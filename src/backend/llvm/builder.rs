use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

use super::{
    context::NativeContext,
    error::NativeError,
    ffi::{
        self, BridgeBinaryOp, BridgeCastOp, BridgeCompareOp, BridgeOverflowOp, BridgeUnaryOp,
        CkcLlvmBlock, CkcLlvmBuilder, CkcLlvmFunction, CkcLlvmType, CkcLlvmValue,
    },
    module::NativeModule,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct NativeType<'context> {
    handle: NonNull<CkcLlvmType>,
    lifetime: PhantomData<&'context NativeContext>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'context> NativeType<'context> {
    fn from_handle(handle: NonNull<CkcLlvmType>) -> Self {
        Self {
            handle,
            lifetime: PhantomData,
            not_send_or_sync: PhantomData,
        }
    }

    pub(super) fn void(context: &'context NativeContext) -> Result<Self, NativeError> {
        ffi::type_void(context.handle()).map(Self::from_handle)
    }

    pub(super) fn int(context: &'context NativeContext, bits: u32) -> Result<Self, NativeError> {
        ffi::type_int(context.handle(), bits).map(Self::from_handle)
    }

    pub(super) fn f64(context: &'context NativeContext) -> Result<Self, NativeError> {
        ffi::type_f64(context.handle()).map(Self::from_handle)
    }

    pub(super) fn pointer(context: &'context NativeContext) -> Result<Self, NativeError> {
        ffi::type_ptr(context.handle()).map(Self::from_handle)
    }

    pub(super) fn slice(context: &'context NativeContext) -> Result<Self, NativeError> {
        ffi::type_slice(context.handle()).map(Self::from_handle)
    }

    pub(super) fn array(element: Self, count: u32) -> Result<Self, NativeError> {
        ffi::type_array(element.handle, count).map(Self::from_handle)
    }

    pub(super) fn fixed_vector(element: Self, count: u32) -> Result<Self, NativeError> {
        ffi::type_fixed_vector(element.handle, count).map(Self::from_handle)
    }

    pub(super) fn literal_struct(
        context: &'context NativeContext,
        fields: &[Self],
    ) -> Result<Self, NativeError> {
        let fields = fields.iter().map(|field| field.handle).collect::<Vec<_>>();
        ffi::type_struct(context.handle(), &fields).map(Self::from_handle)
    }

    pub(super) fn named_struct(
        context: &'context NativeContext,
        name: &str,
    ) -> Result<Self, NativeError> {
        ffi::type_named_struct(context.handle(), name).map(Self::from_handle)
    }

    pub(super) fn set_struct_body(self, fields: &[Self]) -> Result<(), NativeError> {
        let fields = fields.iter().map(|field| field.handle).collect::<Vec<_>>();
        ffi::type_set_struct_body(self.handle, &fields)
    }

    pub(super) const fn handle(self) -> NonNull<CkcLlvmType> {
        self.handle
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct NativeFunction<'module> {
    handle: NonNull<CkcLlvmFunction>,
    lifetime: PhantomData<&'module ()>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'module> NativeFunction<'module> {
    pub(super) const fn handle(self) -> NonNull<CkcLlvmFunction> {
        self.handle
    }

    pub(super) fn param(
        self,
        index: usize,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::function_param(self.handle, index, name).map(NativeValue::from_handle)
    }

    pub(super) fn append_block(self, name: &str) -> Result<NativeBlock<'module>, NativeError> {
        ffi::function_append_block(self.handle, name).map(NativeBlock::from_handle)
    }

    pub(super) fn add_return_extension(
        self,
        extension: super::super::native_abi::NativeAbiExtension,
    ) -> Result<(), NativeError> {
        use super::super::native_abi::NativeAbiExtension;
        let kind = match extension {
            NativeAbiExtension::None => return Ok(()),
            NativeAbiExtension::Zero => ffi::BridgeAttributeKind::ZeroExt,
            NativeAbiExtension::Sign => ffi::BridgeAttributeKind::SignExt,
        };
        ffi::function_add_attribute(self.handle, kind, true, 0, None, 0)
    }

    pub(super) fn add_param_extension(
        self,
        index: usize,
        extension: super::super::native_abi::NativeAbiExtension,
    ) -> Result<(), NativeError> {
        use super::super::native_abi::NativeAbiExtension;
        let kind = match extension {
            NativeAbiExtension::None => return Ok(()),
            NativeAbiExtension::Zero => ffi::BridgeAttributeKind::ZeroExt,
            NativeAbiExtension::Sign => ffi::BridgeAttributeKind::SignExt,
        };
        ffi::function_add_attribute(self.handle, kind, false, index, None, 0)
    }

    pub(super) fn add_sret(
        self,
        index: usize,
        pointee: NativeType<'_>,
        alignment: u32,
    ) -> Result<(), NativeError> {
        ffi::function_add_attribute(
            self.handle,
            ffi::BridgeAttributeKind::Sret,
            false,
            index,
            Some(pointee.handle),
            alignment,
        )
    }

    pub(super) fn add_byval(
        self,
        index: usize,
        pointee: NativeType<'_>,
        alignment: u32,
    ) -> Result<(), NativeError> {
        ffi::function_add_attribute(
            self.handle,
            ffi::BridgeAttributeKind::ByVal,
            false,
            index,
            Some(pointee.handle),
            alignment,
        )
    }

    pub(super) fn add_param_noalias(self, index: usize) -> Result<(), NativeError> {
        ffi::function_add_attribute(
            self.handle,
            ffi::BridgeAttributeKind::NoAlias,
            false,
            index,
            None,
            0,
        )
    }

    pub(super) fn add_param_readonly(self, index: usize) -> Result<(), NativeError> {
        ffi::function_add_attribute(
            self.handle,
            ffi::BridgeAttributeKind::ReadOnly,
            false,
            index,
            None,
            0,
        )
    }

    pub(super) fn add_param_writeonly(self, index: usize) -> Result<(), NativeError> {
        ffi::function_add_attribute(
            self.handle,
            ffi::BridgeAttributeKind::WriteOnly,
            false,
            index,
            None,
            0,
        )
    }

    pub(super) fn add_param_alignment(
        self,
        index: usize,
        alignment: u32,
    ) -> Result<(), NativeError> {
        ffi::function_add_attribute(
            self.handle,
            ffi::BridgeAttributeKind::Align,
            false,
            index,
            None,
            alignment,
        )
    }

    pub(super) fn set_dll_export(self) -> Result<(), NativeError> {
        ffi::function_set_dll_export(self.handle)
    }

    pub(super) fn set_memory_effects(
        self,
        effects: ffi::BridgeMemoryEffects,
    ) -> Result<(), NativeError> {
        ffi::function_set_memory_effects(self.handle, effects)
    }

    pub(super) fn set_profile(
        self,
        entry_count: u64,
        hot: bool,
        cold: bool,
    ) -> Result<(), NativeError> {
        ffi::function_set_profile(self.handle, entry_count, hot, cold)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct NativeBlock<'module> {
    handle: NonNull<CkcLlvmBlock>,
    lifetime: PhantomData<&'module ()>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'module> NativeBlock<'module> {
    fn from_handle(handle: NonNull<CkcLlvmBlock>) -> Self {
        Self {
            handle,
            lifetime: PhantomData,
            not_send_or_sync: PhantomData,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct NativeValue<'module> {
    handle: NonNull<CkcLlvmValue>,
    lifetime: PhantomData<&'module ()>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'module> NativeValue<'module> {
    fn from_handle(handle: NonNull<CkcLlvmValue>) -> Self {
        Self {
            handle,
            lifetime: PhantomData,
            not_send_or_sync: PhantomData,
        }
    }
}

impl NativeModule<'_> {
    pub(super) fn add_function<'module>(
        &'module self,
        name: &str,
        return_type: NativeType<'_>,
        params: &[NativeType<'_>],
        exported: bool,
    ) -> Result<NativeFunction<'module>, NativeError> {
        let params = params
            .iter()
            .map(|param| param.handle())
            .collect::<Vec<_>>();
        let handle = ffi::module_add_function(
            self.shared_handle(),
            name,
            return_type.handle(),
            &params,
            exported,
        )?;
        Ok(NativeFunction {
            handle,
            lifetime: PhantomData,
            not_send_or_sync: PhantomData,
        })
    }

    pub(super) fn preserve_function(
        &self,
        function: NativeFunction<'_>,
    ) -> Result<(), NativeError> {
        ffi::module_preserve_function(self.shared_handle(), function.handle())
    }

    pub(super) fn add_global_bytes<'module>(
        &'module self,
        name: &str,
        bytes: &[u8],
        mutable_storage: bool,
        alignment: u32,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::module_add_global_bytes(
            self.shared_handle(),
            name,
            bytes,
            mutable_storage,
            alignment,
        )
        .map(NativeValue::from_handle)
    }

    pub(super) fn add_global_u32_array<'module>(
        &'module self,
        name: &str,
        values: &[u32],
        alignment: u32,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::module_add_global_u32_array(self.shared_handle(), name, values, alignment)
            .map(NativeValue::from_handle)
    }
}

pub(super) struct NativeBuilder<'module, 'context> {
    handle: NonNull<CkcLlvmBuilder>,
    module: PhantomData<&'module NativeModule<'context>>,
    context: &'context NativeContext,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'module, 'context> NativeBuilder<'module, 'context> {
    pub(super) fn new(
        context: &'context NativeContext,
        _module: &'module NativeModule<'context>,
    ) -> Result<Self, NativeError> {
        Ok(Self {
            handle: ffi::builder_create(context.handle())?,
            module: PhantomData,
            context,
            not_send_or_sync: PhantomData,
        })
    }

    pub(super) fn position(&mut self, block: NativeBlock<'module>) -> Result<(), NativeError> {
        ffi::builder_position(self.handle, block.handle)
    }

    pub(super) fn alloca(
        &mut self,
        type_node: NativeType<'context>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_alloca(self.handle, type_node.handle(), name).map(NativeValue::from_handle)
    }

    pub(super) fn load(
        &mut self,
        type_node: NativeType<'context>,
        pointer: NativeValue<'module>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_load(self.handle, type_node.handle(), pointer.handle, name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn load_scoped_alias(
        &mut self,
        type_node: NativeType<'context>,
        pointer: NativeValue<'module>,
        alias_scopes: &[u32],
        noalias_scopes: &[u32],
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_load_scoped_alias(
            self.handle,
            type_node.handle(),
            pointer.handle,
            alias_scopes,
            noalias_scopes,
            name,
        )
        .map(NativeValue::from_handle)
    }

    pub(super) fn store(
        &mut self,
        value: NativeValue<'module>,
        pointer: NativeValue<'module>,
    ) -> Result<(), NativeError> {
        ffi::builder_store(self.handle, value.handle, pointer.handle)
    }

    pub(super) fn store_scoped_alias(
        &mut self,
        value: NativeValue<'module>,
        pointer: NativeValue<'module>,
        alias_scopes: &[u32],
        noalias_scopes: &[u32],
    ) -> Result<(), NativeError> {
        ffi::builder_store_scoped_alias(
            self.handle,
            value.handle,
            pointer.handle,
            alias_scopes,
            noalias_scopes,
        )
    }

    pub(super) fn vector_load(
        &mut self,
        type_node: NativeType<'context>,
        pointer: NativeValue<'module>,
        alignment: u32,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_vector_load(
            self.handle,
            type_node.handle(),
            pointer.handle,
            alignment,
            name,
        )
        .map(NativeValue::from_handle)
    }

    pub(super) fn vector_store(
        &mut self,
        value: NativeValue<'module>,
        pointer: NativeValue<'module>,
        alignment: u32,
    ) -> Result<(), NativeError> {
        ffi::builder_vector_store(self.handle, value.handle, pointer.handle, alignment)
    }

    pub(super) fn const_int(
        &self,
        type_node: NativeType<'context>,
        text: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::const_int(type_node.handle(), text).map(NativeValue::from_handle)
    }

    pub(super) fn const_float(
        &self,
        type_node: NativeType<'context>,
        text: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::const_float(type_node.handle(), text).map(NativeValue::from_handle)
    }

    pub(super) fn const_bool(&self, value: bool) -> Result<NativeValue<'module>, NativeError> {
        ffi::const_bool(self.context.handle(), value).map(NativeValue::from_handle)
    }

    pub(super) fn undef(
        &self,
        type_node: NativeType<'context>,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::const_undef(type_node.handle()).map(NativeValue::from_handle)
    }

    pub(super) fn binary(
        &mut self,
        op: BridgeBinaryOp,
        left: NativeValue<'module>,
        right: NativeValue<'module>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        self.binary_with_flags(op, left, right, false, false, name)
    }

    pub(super) fn binary_with_flags(
        &mut self,
        op: BridgeBinaryOp,
        left: NativeValue<'module>,
        right: NativeValue<'module>,
        no_unsigned_wrap: bool,
        no_signed_wrap: bool,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_binary(
            self.handle,
            op,
            left.handle,
            right.handle,
            no_unsigned_wrap,
            no_signed_wrap,
            name,
        )
        .map(NativeValue::from_handle)
    }

    pub(super) fn overflow(
        &mut self,
        op: BridgeOverflowOp,
        left: NativeValue<'module>,
        right: NativeValue<'module>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_overflow(self.handle, op, left.handle, right.handle, name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn unary(
        &mut self,
        op: BridgeUnaryOp,
        value: NativeValue<'module>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_unary(self.handle, op, value.handle, name).map(NativeValue::from_handle)
    }

    pub(super) fn compare(
        &mut self,
        op: BridgeCompareOp,
        left: NativeValue<'module>,
        right: NativeValue<'module>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_compare(self.handle, op, left.handle, right.handle, name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn cast(
        &mut self,
        op: BridgeCastOp,
        value: NativeValue<'module>,
        target_type: NativeType<'context>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_cast(self.handle, op, value.handle, target_type.handle(), name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn gep(
        &mut self,
        element_type: NativeType<'context>,
        pointer: NativeValue<'module>,
        indices: &[NativeValue<'module>],
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        let indices = indices.iter().map(|index| index.handle).collect::<Vec<_>>();
        ffi::builder_gep(
            self.handle,
            element_type.handle(),
            pointer.handle,
            &indices,
            name,
        )
        .map(NativeValue::from_handle)
    }

    pub(super) fn extract_value(
        &mut self,
        aggregate: NativeValue<'module>,
        index: u32,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_extract_value(self.handle, aggregate.handle, index, name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn insert_value(
        &mut self,
        aggregate: NativeValue<'module>,
        value: NativeValue<'module>,
        index: u32,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_insert_value(self.handle, aggregate.handle, value.handle, index, name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn select(
        &mut self,
        condition: NativeValue<'module>,
        then_value: NativeValue<'module>,
        else_value: NativeValue<'module>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_select(
            self.handle,
            condition.handle,
            then_value.handle,
            else_value.handle,
            name,
        )
        .map(NativeValue::from_handle)
    }

    pub(super) fn vector_splat(
        &mut self,
        lanes: u32,
        scalar: NativeValue<'module>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_vector_splat(self.handle, lanes, scalar.handle, name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn vector_insert(
        &mut self,
        vector: NativeValue<'module>,
        scalar: NativeValue<'module>,
        lane_index: u32,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_vector_insert(self.handle, vector.handle, scalar.handle, lane_index, name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn vector_extract(
        &mut self,
        vector: NativeValue<'module>,
        lane_index: u32,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_vector_extract(self.handle, vector.handle, lane_index, name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn vector_reduce(
        &mut self,
        reduction: u32,
        vector: NativeValue<'module>,
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        ffi::builder_vector_reduce(self.handle, reduction, vector.handle, name)
            .map(NativeValue::from_handle)
    }

    pub(super) fn assume(&mut self, condition: NativeValue<'module>) -> Result<(), NativeError> {
        ffi::builder_assume(self.handle, condition.handle)
    }

    pub(super) fn call(
        &mut self,
        function: NativeFunction<'module>,
        args: &[NativeValue<'module>],
        name: &str,
    ) -> Result<NativeValue<'module>, NativeError> {
        let args = args.iter().map(|arg| arg.handle).collect::<Vec<_>>();
        ffi::builder_call(self.handle, function.handle, &args, name).map(NativeValue::from_handle)
    }

    pub(super) fn return_void(&mut self) -> Result<(), NativeError> {
        ffi::builder_return_void(self.handle)
    }

    pub(super) fn return_value(&mut self, value: NativeValue<'module>) -> Result<(), NativeError> {
        ffi::builder_return(self.handle, value.handle)
    }

    pub(super) fn branch(&mut self, target: NativeBlock<'module>) -> Result<(), NativeError> {
        ffi::builder_branch(self.handle, target.handle)
    }

    pub(super) fn cond_branch(
        &mut self,
        condition: NativeValue<'module>,
        then_block: NativeBlock<'module>,
        else_block: NativeBlock<'module>,
    ) -> Result<(), NativeError> {
        ffi::builder_cond_branch(
            self.handle,
            condition.handle,
            then_block.handle,
            else_block.handle,
        )
    }

    pub(super) fn cond_branch_weighted(
        &mut self,
        condition: NativeValue<'module>,
        then_block: NativeBlock<'module>,
        else_block: NativeBlock<'module>,
        then_count: u64,
        else_count: u64,
    ) -> Result<(), NativeError> {
        ffi::builder_cond_branch_weighted(
            self.handle,
            condition.handle,
            then_block.handle,
            else_block.handle,
            then_count,
            else_count,
        )
    }
}

impl Drop for NativeBuilder<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: This wrapper is the unique owner of the bridge builder.
        unsafe { ffi::builder_dispose(self.handle) };
    }
}
