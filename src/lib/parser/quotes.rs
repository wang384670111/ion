use std::{iter::Peekable, str};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Quotes {
    Single,
    Double,
    None,
}

/// Serves as a buffer for storing a string until that string can be terminated.
/// A string terminates if
///
/// - It reaches the end without finding a new line
/// - It reaches a newline without a "\\" char before it
///
/// This example comes from the shell's REPL, which ensures that the user's input
/// will only be submitted for execution once a terminated command is supplied.
#[derive(Debug)]
pub struct Terminator<I: Iterator<Item = u8>> {
    inner:      RearPeekable<I>,
    array:      usize,
    skip_next:  bool,
    quotes:     Quotes,
    terminated: bool,
    and_or:     bool,
    whitespace: bool,
    empty:      bool,
    subshell:   usize,
}

impl<'a> From<&'a str> for Terminator<std::str::Bytes<'a>> {
    fn from(string: &'a str) -> Self { Self::new(string.bytes()) }
}

#[derive(Clone, Debug)]
pub struct RearPeekable<I: Iterator> {
    iter: Peekable<I>,
    now:  Option<I::Item>,
    last: Option<I::Item>,
}

impl<I> Iterator for RearPeekable<I>
where
    I: Iterator,
    I::Item: Copy,
{
    type Item = I::Item;

    #[inline]
    fn next(&mut self) -> Option<I::Item> {
        self.last = self.now;
        self.now = self.iter.next();
        self.now
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) { self.iter.size_hint() }
}

impl<I: Iterator> RearPeekable<I> {
    #[inline]
    pub fn peek(&mut self) -> Option<&I::Item> { self.iter.peek() }

    #[inline]
    pub fn prev(&self) -> Option<&I::Item> { self.last.as_ref() }
}

impl<I: Iterator<Item = u8>> Iterator for Terminator<I> {
    type Item = u8;

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) { self.inner.size_hint() }

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.terminated {
            return None;
        }

        let prev_whitespace = self.whitespace;
        self.whitespace = false;

        let mut next = if prev_whitespace && self.array == 0 && !self.and_or && !self.empty {
            self.inner.find(|&c| c == b'\n' || !c.is_ascii_whitespace())
        } else if prev_whitespace {
            self.inner.find(|&c| !c.is_ascii_whitespace())
        } else {
            self.inner.next()
        };

        if self.skip_next {
            self.skip_next = false;
        } else if self.quotes != Quotes::None && next != Some(b'\\') {
            match (next, &self.quotes) {
                (Some(b'\''), Quotes::Single) | (Some(b'"'), Quotes::Double) => {
                    self.quotes = Quotes::None;
                }
                _ => (),
            }
        } else if let Some(character) = next {
            next = self.handle_char(character, prev_whitespace);
            self.empty &= character.is_ascii_whitespace();
        } else if self.subshell == 0 && self.array == 0 && !self.and_or && !self.empty {
            self.terminated = true;
        }

        next
    }
}

impl<I: Iterator<Item = u8>> Terminator<I> {
    /// Consumes lines until a statement is formed or the iterator runs dry, and returns the
    /// underlying `String`.
    ///
    /// TODO: Fix strange trimming or remove inconsistent trimming
    /// Trimming white space from left and right is strange
    /// Trimming from left reduces to one space and trimming from right is only done if no white
    /// spaces are found from left.
    /// TODO: Comments and empty/white space only lines should not cause yielding an
    /// empty/white-space line.
    pub fn terminate(&mut self) -> Option<String> {
        let stmt = self.collect::<Vec<_>>();
        let stmt = unsafe { String::from_utf8_unchecked(stmt) };

        if self.empty {
            None
        } else {
            Some(stmt)
        }
    }

    fn handle_char(&mut self, character: u8, prev_whitespace: bool) -> Option<u8> {
        match character {
            b'\'' => {
                self.quotes = Quotes::Single;
                Some(b'\'')
            }
            b'"' => {
                self.quotes = Quotes::Double;
                Some(b'"')
            }
            b'(' if self.inner.prev() == Some(&b'$') || self.inner.prev() == Some(&b'@') => {
                self.subshell += 1;
                Some(b'(')
            }
            b')' if self.subshell > 0 => {
                self.subshell -= 1;
                Some(b')')
            }
            b'[' => {
                self.array += 1;
                Some(b'[')
            }
            b']' if self.array > 0 => {
                self.array -= 1;
                Some(b']')
            }
            b'#' if prev_whitespace || self.inner.prev().is_none() => {
                self.inner.find(|&c| c == b'\n');
                if self.array == 0 && self.subshell == 0 && !self.and_or && !self.empty {
                    self.terminated = true;
                    None
                } else {
                    self.whitespace = true;
                    Some(b' ')
                }
            }
            b'\\' => {
                if self.inner.peek() == Some(&b'\n') {
                    self.whitespace = true;
                    self.inner.next();
                    self.next()
                } else {
                    self.skip_next = true;
                    Some(character)
                }
            }
            b'&' | b'|' if self.inner.prev() == Some(&character) => {
                self.and_or = true;
                Some(character)
            }
            b'\n' if self.array == 0 && self.subshell == 0 && !self.and_or && !self.empty => {
                self.terminated = true;
                None
            }
            _ if character.is_ascii_whitespace() => {
                self.whitespace = true;
                Some(b' ')
            }
            _ => {
                self.and_or = false;
                Some(character)
            }
        }
    }

    /// Create a new reader on the provided input
    pub fn new(inner: I) -> Self {
        Self {
            inner:      RearPeekable { iter: inner.peekable(), now: None, last: None },
            array:      0,
            skip_next:  false,
            quotes:     Quotes::None,
            terminated: false,
            and_or:     false,
            whitespace: false,
            empty:      true,
            subshell:   0,
        }
    }
}

#[cfg(test)]
mod testing {
    use itertools::Itertools;

    use super::*;
    #[test]
    fn should_terminate_to_new_line() {
        assert_case("echo hello", Some("echo hello".to_owned()));
        assert_case("a", Some("a".to_owned()));
        assert_case(
            "echo hello
             echo world",
            Some("echo hello".to_owned()),
        );
        assert_case(
            "echo hello;echo all
             echo world",
            Some("echo hello;echo all".to_owned()),
        );
        assert_case(
            "echo hello\\
             echo all
             echo world",
            Some("echo helloecho all".to_owned()),
        );
        assert_case(
            "echo hello\\
             echo all
             echo world",
            Some("echo helloecho all".to_owned()),
        );
        assert_case("", None);

        fn assert_case(input: &str, expected: Option<String>) {
            let actual = Terminator::new(input.bytes()).terminate();
            assert_eq!(actual, expected, "Should have terminated to {:?}", expected);
        }
    }

    #[test]
    fn should_terminate_all_items() {
        let left_input = "fn greet\n  echo hi there\nend\n greet  \n\n#  Some comments\n # \
                          another comment\necho \"out there\"\n#  another comment";

        let stmts = left_input
            .bytes()
            .batching(|lines| Terminator::new(lines).terminate())
            .collect::<Vec<_>>();

        assert_eq!(
            vec![
                "fn greet".to_owned(),
                // TODO: not trimming left of spaces and only one space before ?
                " echo hi there".to_owned(),
                // TODO: Triming from right is donw however ?
                "end".to_owned(),
                // TODO: is not even trimmed from right if it hases space from both sides ?
                " greet ".to_owned(),
                // TODO: comments and white-space/empt lines are yielded as white space only
                // lines ?.
                "  ".to_owned(),
                "echo \"out there\"".to_owned(),
                " ".to_owned(),
            ],
            stmts,
        );
    }
}
