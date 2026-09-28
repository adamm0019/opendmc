/// `*` matches any run of characters, `?` exactly one.
pub fn matches(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            backtrack = Some((pi, ti));
            pi += 1;
        } else if let Some((bp, bt)) = backtrack {
            pi = bp + 1;
            ti = bt + 1;
            backtrack = Some((bp, bt + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn wildcards() {
        assert!(matches("*.pld", "pl00.pld"));
        assert!(matches("pl0?.pld", "pl05.pld"));
        assert!(matches("*", ""));
        assert!(matches("em*_*.emd", "em2_x.emd"));
        assert!(!matches("*.pld", "pl00.emd"));
        assert!(!matches("pl?", "pl"));
    }
}
