//! Pure behavior for the `echo` applet.

/// Write the existing `echo` byte stream through `write`.
///
/// The applet deliberately treats every argument literally: it does not
/// recognize options or escapes, and it ignores write errors just as the
/// original multicall implementation did.
pub fn run(args: &[&str], write: &mut dyn FnMut(&[u8]) -> Result<(), ()>) -> i32 {
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            let _ = write(b" ");
        }
        let _ = write(arg.as_bytes());
    }
    let _ = write(b"\n");
    0
}

/// Remove the executable name from a native argv slice.
pub fn user_args<'a>(argv: &'a [&'a str]) -> &'a [&'a str] {
    argv.get(1..).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::string::String;
    use std::vec::Vec;

    fn render(args: &[&str]) -> (String, i32) {
        let mut output = Vec::new();
        let status = run(args, &mut |bytes| {
            output.extend_from_slice(bytes);
            Ok(())
        });
        (String::from_utf8(output).unwrap(), status)
    }

    #[test]
    fn preserves_literal_arguments_and_newline() {
        let cases = [
            (&[][..], "\n"),
            (&["one"][..], "one\n"),
            (&["one", "two"][..], "one two\n"),
            (&["", "-n", "a\\nb", "sun☀"][..], " -n a\\nb sun☀\n"),
        ];

        for (args, expected) in cases {
            let (actual, status) = render(args);
            assert_eq!(actual, expected);
            assert_eq!(status, 0);
        }
    }

    #[test]
    fn ignores_write_failures_and_keeps_success_status() {
        let mut calls = 0;
        let status = run(&["x"], &mut |_| {
            calls += 1;
            Err(())
        });

        assert_eq!(calls, 2);
        assert_eq!(status, 0);
    }

    #[test]
    fn excludes_argv_zero_from_echo_arguments() {
        let empty: &[&str] = &[];
        assert_eq!(user_args(empty), empty);
        assert_eq!(user_args(&["echo"]), empty);
        assert_eq!(user_args(&["echo", "Hi"]), &["Hi"]);
        assert_eq!(user_args(&["echo", "Hi", "there"]), &["Hi", "there"]);
    }
}
