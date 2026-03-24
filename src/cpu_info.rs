//! Read CPU marketing name from `/proc/cpuinfo` for friendlier panel titles than sensor labels alone.

use std::fs;

/// First `model name` (x86) or `Model` (some ARM) line, normalized whitespace.
pub fn read_cpu_model_from_proc() -> Option<String> {
    let data = fs::read_to_string("/proc/cpuinfo").ok()?;
    parse_model_name(&data)
}

fn parse_model_name(cpuinfo: &str) -> Option<String> {
    for line in cpuinfo.lines() {
        let line = line.trim();
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();
        if (key == "model name" || key == "Model") && !val.is_empty() {
            return Some(squeeze_spaces(val));
        }
    }
    None
}

fn squeeze_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_name_tab() {
        let s = "processor\t: 0\nmodel name\t: AMD Ryzen 9 9950X 16-Core Processor\n";
        assert_eq!(
            parse_model_name(s).as_deref(),
            Some("AMD Ryzen 9 9950X 16-Core Processor")
        );
    }

    #[test]
    fn parses_model_name_space() {
        let s = "model name : Intel(R) Core(TM) i7-1065G7 CPU @ 1.30GHz\n";
        assert!(parse_model_name(s)
            .unwrap()
            .contains("Intel(R) Core(TM) i7-1065G7"));
    }
}
