pub(crate) const FIXED_SEED: u64 = 0xC0DE_CAFE_5EED_0110;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeneratedKernelCase {
    pub(crate) function: String,
    pub(crate) values: [i32; 8],
    pub(crate) len: u32,
    pub(crate) bias: i32,
    pub(crate) expected: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeneratedKernelProgram {
    pub(crate) source: String,
    pub(crate) cases: Vec<GeneratedKernelCase>,
}

pub(crate) fn fixed_seed_kernel_program() -> GeneratedKernelProgram {
    let mut state = FIXED_SEED;
    let mut source = String::new();
    let mut cases = Vec::new();

    for index in 0..3 {
        let weight = next_in(&mut state, 2, 5) as i32;
        let offset = next_in(&mut state, 1, 7) as i32;
        let skip = next_in(&mut state, 1, 2);
        let stop = next_in(&mut state, 4, 5);
        let len = 6;
        let bias = next_in(&mut state, 3, 17) as i32;
        let mut values = [0; 8];
        for value in &mut values {
            *value = next_in(&mut state, 1, 19) as i32;
        }

        let function = format!("generated_kernel_{index}");
        source.push_str(&format!(
            r#"
export unsafe fn {function}(items: slice<i32>, len: u32, bias: i32) -> i32
contract {{ requires len <= items.len; effects read(items); }}
{{
  let i: u32 = 0;
  let total: i32 = bias;
  while i < len {{
    if i == {skip} {{ i = i + 1; continue; }}
    total = total + items[i] * {weight} + {offset};
    if i == {stop} {{ break; }}
    i = i + 1;
  }}
  return total;
}}
"#
        ));

        let mut expected = bias;
        let mut i = 0_u32;
        while i < len {
            if i == skip {
                i += 1;
                continue;
            }
            expected += values[i as usize] * weight + offset;
            if i == stop {
                break;
            }
            i += 1;
        }
        cases.push(GeneratedKernelCase {
            function,
            values,
            len,
            bias,
            expected,
        });
    }

    GeneratedKernelProgram { source, cases }
}

fn next_in(state: &mut u64, low: u32, high: u32) -> u32 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    low + ((*state >> 32) as u32 % (high - low + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_kernel_fixture_should_be_byte_deterministic_and_contract_valid() {
        let first = fixed_seed_kernel_program();
        let second = fixed_seed_kernel_program();
        assert_eq!(first, second);
        assert_eq!(first.cases.len(), 3);
        assert!(first.source.contains("requires len <= items.len"));
        assert!(
            first
                .cases
                .iter()
                .all(|case| case.len <= case.values.len() as u32)
        );
    }
}
