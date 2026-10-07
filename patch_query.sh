sed -i '' -e '36,45c\
pub fn admit_memory(query: &BrainQuery, memory: &Memory, policy: &AdmissionPolicy) -> bool {\
    let memory_text = memory.text.to_ascii_lowercase();\
    \
    if policy.strict_mode {\
        if query.confirmed_subjects.is_empty() {\
            return false;\
        }\
        let mut match_found = false;\
        for subject in &query.confirmed_subjects {\
            if memory_text.contains(&subject.to_ascii_lowercase()) {\
                match_found = true;\
                break;\
            }\
        }\
        if !match_found {\
            return false;\
        }\
    }\
    true\
}\
\
pub fn pack_context(_admitted: &[Memory], _budget: usize) -> String {\
    String::new()\
}
