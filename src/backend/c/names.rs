use std::collections::HashSet;

#[derive(Debug, Default)]
pub(super) struct CIdentifierAllocator {
    used: HashSet<String>,
}

impl CIdentifierAllocator {
    pub(super) fn reserve(&mut self, name: &str) {
        self.used.insert(name.to_string());
    }

    pub(super) fn allocate(&mut self, preferred: &str) -> String {
        if self.used.insert(preferred.to_string()) {
            return preferred.to_string();
        }
        let mut suffix = 1_u32;
        loop {
            let candidate = format!("{preferred}_{suffix}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
            suffix += 1;
        }
    }
}

pub(super) fn sanitize_c_identifier(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}
