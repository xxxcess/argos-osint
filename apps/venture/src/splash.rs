//! Welcome mark for an interactive Venture session.

pub(crate) const MARK: &str = r#"
                 △
            △    △    △
         △                 △

              .-""""""-.
            .'          '.
           /              \
          |                |
           \              /
            '.          .'
              '-......-'
            .'          '.
          .'              '.
        .'                  '.
         '-.______________.-'

             F L E E T W O R K S
"#;

#[cfg(test)]
mod tests {
    use super::MARK;

    #[test]
    fn welcome_mark_names_fleetworks_and_keeps_the_rays() {
        assert!(MARK.contains("F L E E T W O R K S"));
        assert!(MARK.matches('△').count() >= 5);
    }
}
