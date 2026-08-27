use super::*;

impl<'module> FunctionLowerer<'module, '_, '_> {
    pub(super) fn status_abi(&self) -> bool {
        status_abi(self.options)
    }

    pub(super) fn status(&self, value: i32) -> Result<NativeValue<'module>, NativeError> {
        self.builder.const_int(self.types.i32, &value.to_string())
    }

    pub(super) fn guard_with_status(
        &mut self,
        failed: NativeValue<'module>,
        status: NativeValue<'module>,
    ) -> Result<(), NativeError> {
        let failure_name = self.next_name("checked.failure");
        let continue_name = self.next_name("checked.continue");
        let failure = self.handle.append_block(&failure_name)?;
        let continuation = self.handle.append_block(&continue_name)?;
        self.builder.cond_branch(failed, failure, continuation)?;
        self.builder.position(failure)?;
        self.builder.return_value(status)?;
        self.builder.position(continuation)
    }

    pub(super) fn checked_binary(
        &mut self,
        op: MirBinaryOp,
        type_node: &MirType,
        left: NativeValue<'module>,
        right: NativeValue<'module>,
    ) -> Result<NativeValue<'module>, NativeError> {
        if matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64)) {
            let name = self.next_name("binary");
            return self
                .builder
                .binary(binary_op(op, type_node)?, left, right, &name);
        }

        let unsigned = matches!(
            type_node,
            MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
        );
        match op {
            MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul => {
                let overflow_op = match (op, unsigned) {
                    (MirBinaryOp::Add, false) => BridgeOverflowOp::SignedAdd,
                    (MirBinaryOp::Add, true) => BridgeOverflowOp::UnsignedAdd,
                    (MirBinaryOp::Sub, false) => BridgeOverflowOp::SignedSub,
                    (MirBinaryOp::Sub, true) => BridgeOverflowOp::UnsignedSub,
                    (MirBinaryOp::Mul, false) => BridgeOverflowOp::SignedMul,
                    (MirBinaryOp::Mul, true) => BridgeOverflowOp::UnsignedMul,
                    _ => unreachable!(),
                };
                let name = self.next_name("overflow.pair");
                let pair = self.builder.overflow(overflow_op, left, right, &name)?;
                let name = self.next_name("overflow.value");
                let result = self.builder.extract_value(pair, 0, &name)?;
                let name = self.next_name("overflow.flag");
                let overflowed = self.builder.extract_value(pair, 1, &name)?;
                let status = self.status(1)?;
                self.guard_with_status(overflowed, status)?;
                Ok(result)
            }
            MirBinaryOp::Div | MirBinaryOp::Mod => {
                let integer_type = self.types.get(type_node)?;
                let zero = self.builder.const_int(integer_type, "0")?;
                let name = self.next_name("division.by_zero");
                let by_zero = self
                    .builder
                    .compare(BridgeCompareOp::IcmpEq, right, zero, &name)?;
                let status = self.status(2)?;
                self.guard_with_status(by_zero, status)?;

                if !unsigned {
                    let min = match type_node {
                        MirType::Primitive(MirPrimitiveTypeName::I32) => "-2147483648",
                        MirType::Primitive(MirPrimitiveTypeName::I64) => "-9223372036854775808",
                        _ => {
                            return Err(lowering_error(
                                "checked signed division requires i32 or i64",
                            ));
                        }
                    };
                    let min = self.builder.const_int(integer_type, min)?;
                    let negative_one = self.builder.const_int(integer_type, "-1")?;
                    let name = self.next_name("division.minimum");
                    let is_min = self
                        .builder
                        .compare(BridgeCompareOp::IcmpEq, left, min, &name)?;
                    let name = self.next_name("division.negative_one");
                    let is_negative_one = self.builder.compare(
                        BridgeCompareOp::IcmpEq,
                        right,
                        negative_one,
                        &name,
                    )?;
                    let false_value = self.builder.const_bool(false)?;
                    let name = self.next_name("division.overflows");
                    let overflows =
                        self.builder
                            .select(is_min, is_negative_one, false_value, &name)?;
                    let status = self.status(1)?;
                    self.guard_with_status(overflows, status)?;
                }
                let name = self.next_name("binary");
                self.builder
                    .binary(binary_op(op, type_node)?, left, right, &name)
            }
        }
    }

    pub(super) fn checked_unary(
        &mut self,
        op: MirUnaryOp,
        type_node: &MirType,
        operand: NativeValue<'module>,
    ) -> Result<NativeValue<'module>, NativeError> {
        if op == MirUnaryOp::Not
            || matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64))
        {
            let name = self.next_name("unary");
            return self.builder.unary(unary_op(op, type_node), operand, &name);
        }
        let integer_type = self.types.get(type_node)?;
        if matches!(
            type_node,
            MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
        ) {
            let zero = self.builder.const_int(integer_type, "0")?;
            let name = self.next_name("negate.pair");
            let pair =
                self.builder
                    .overflow(BridgeOverflowOp::UnsignedSub, zero, operand, &name)?;
            let name = self.next_name("negate.value");
            let result = self.builder.extract_value(pair, 0, &name)?;
            let name = self.next_name("negate.overflow");
            let overflowed = self.builder.extract_value(pair, 1, &name)?;
            let status = self.status(1)?;
            self.guard_with_status(overflowed, status)?;
            return Ok(result);
        }
        let minimum = match type_node {
            MirType::Primitive(MirPrimitiveTypeName::I32) => "-2147483648",
            MirType::Primitive(MirPrimitiveTypeName::I64) => "-9223372036854775808",
            _ => return Err(lowering_error("checked negation requires an integer")),
        };
        let minimum = self.builder.const_int(integer_type, minimum)?;
        let name = self.next_name("negate.minimum");
        let overflowed = self
            .builder
            .compare(BridgeCompareOp::IcmpEq, operand, minimum, &name)?;
        let status = self.status(1)?;
        self.guard_with_status(overflowed, status)?;
        let name = self.next_name("unary");
        self.builder.unary(BridgeUnaryOp::Neg, operand, &name)
    }
}
