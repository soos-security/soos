//! Additive static scanners for `scripts/setup.sh` (GitHub #459, part of #362; follow-up of the
//! reviews of PR #440 and PR #456; rows SCN1-SCN5; walkthrough 245).
//!
//! SET40 (`setup_contract.rs`) checks the daemon unit bound line by line, so a `systemctl`
//! call split with a backslash continuation escapes it; SET41 only sees a `tailscale` word that
//! is not preceded by `/` and is followed by a blank, so `"$(/usr/bin/tailscale serve)"`,
//! `$("tailscale" serve)` and a Tailscale command held in a variable escape it. Those tests are
//! left byte-identical; this module adds a small quote-, substitution- and here-document-aware
//! shell lexer that joins continuation lines into one simple command, and new checks on top of
//! it. Every check is first run on synthetic bad input (it must fail) and good input (it must
//! pass), then on the real `scripts/setup.sh`.
//!
//! Rows SCN6-SCN12 (walkthrough 251, follow-up of the review of PR #460) harden the lexer: a
//! `tailscale` word spelled through quotes or escapes, command words taken from variables the
//! environment could set, arithmetic `(( x << 2 ))`, `[[ ... ]]` tests and `case ... esac`
//! inside `$( ... )`; `try_lex` never panics and returns an `Err` the checks fail on.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

const SETUP: &str = "scripts/setup.sh";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read_repo(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

// ---------------------------------------------------------------------------
// Shell lexer
// ---------------------------------------------------------------------------

/// One simple command: its raw words (quotes and expansions kept verbatim, every backslash
/// continuation removed), whether it runs inside a command or process substitution at any
/// depth, and the line of its first word.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShCmd {
    words: Vec<String>,
    in_subst: bool,
    line: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Term {
    /// The whole script.
    Top,
    /// `$( ... )`, `<( ... )` or `>( ... )`.
    Paren,
    /// `` `...` ``
    Backtick,
}

/// Deepest nesting of substitutions, quotes and expansions the lexer follows. Deeper input is
/// refused (an `Err`, so the check fails closed) instead of overflowing the stack.
const MAX_NESTING: usize = 64;

/// Result of a lexer step: the index after what was read, or why the script cannot be read.
type Lx = Result<usize, String>;

/// The words of the simple command being read, and the `case ... esac` state of its list.
#[derive(Default)]
struct Acc {
    words: Vec<String>,
    word: String,
    started: bool,
    first: Option<usize>,
    /// `case ... in` opened in this list and not yet closed by `esac`.
    case_depth: usize,
    /// The next words are a `case` pattern list, ended by `)`.
    pattern: bool,
    /// Inside `[[ ... ]]`, where `&&`, `||`, `(` and newlines are part of the test.
    test: bool,
}

impl Acc {
    fn push(&mut self, at: usize, text: &str) {
        if self.first.is_none() {
            self.first = Some(at);
        }
        self.started = true;
        self.word.push_str(text);
    }

    fn end_word(&mut self) {
        if self.started {
            self.words.push(std::mem::take(&mut self.word));
            self.started = false;
            let n = self.words.len();
            if n == 3 && self.words[0] == "case" && self.words[2] == "in" {
                self.case_depth += 1;
                self.pattern = true;
            } else if n == 1 && self.words[0] == "esac" && self.case_depth > 0 {
                self.case_depth -= 1;
                self.pattern = false;
            }
            // `[[` opens a test only in command-word position (`echo [[` is an argument).
            let keywords = [
                "if", "elif", "while", "until", "!", "then", "do", "else", "{", "time",
            ];
            match self.words.split_last() {
                Some((last, before))
                    if last == "[["
                        && !self.test
                        && before.iter().all(|w| keywords.contains(&w.as_str())) =>
                {
                    self.test = true;
                }
                Some((last, _)) if last == "]]" && self.test => self.test = false,
                _ => {}
            }
        }
    }

    /// Inside an open `[[ ... ]]` test, the closing `]]` being read not counted as inside.
    fn in_test(&self) -> bool {
        self.test && !(self.started && self.word == "]]")
    }

    /// Whether the word being read is a command-position `esac` (it closes the `case`, it is
    /// not a pattern).
    fn at_esac(&self) -> bool {
        self.started && self.words.is_empty() && self.word == "esac"
    }
}

struct Lexer<'a> {
    chars: &'a [char],
    /// 1-based line of every char index.
    line_of: Vec<usize>,
    cmds: Vec<ShCmd>,
    /// Here-documents opened on the current line: (delimiter, expansions active).
    pending: Vec<(String, bool)>,
    /// Current nesting depth (bounded by `MAX_NESTING`).
    depth: usize,
    /// Start indexes after `((` already found not to be arithmetic (read once only).
    not_arith: std::collections::HashSet<usize>,
}

impl Lexer<'_> {
    fn line(&self, at: usize) -> usize {
        self.line_of
            .get(at)
            .copied()
            .unwrap_or_else(|| self.line_of.last().copied().unwrap_or(1))
    }

    /// The text of `chars[start..end]` without backslash continuations.
    fn raw(&self, start: usize, end: usize) -> String {
        let mut out = String::new();
        let mut i = start;
        while i < end.min(self.chars.len()) {
            if self.chars[i] == '\\' && self.chars.get(i + 1) == Some(&'\n') {
                i += 2;
                continue;
            }
            out.push(self.chars[i]);
            i += 1;
        }
        out
    }

    /// Runs `step` one nesting level deeper, refusing input nested deeper than `MAX_NESTING`.
    fn nested<T>(
        &mut self,
        at: usize,
        step: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.depth >= MAX_NESTING {
            return Err(format!(
                "nesting deeper than {MAX_NESTING} levels on line {}",
                self.line(at)
            ));
        }
        self.depth += 1;
        let result = step(self);
        self.depth -= 1;
        result
    }

    fn end_cmd(&mut self, acc: &mut Acc, in_subst: bool) {
        acc.end_word();
        if !acc.words.is_empty() {
            let line = self.line(acc.first.unwrap_or(0));
            self.cmds.push(ShCmd {
                words: std::mem::take(&mut acc.words),
                in_subst,
                line,
            });
        }
        acc.first = None;
    }

    /// `'...'` starting after the opening quote; returns the index after the closing quote.
    fn single(&self, mut i: usize, opened: usize) -> Lx {
        while i < self.chars.len() {
            if self.chars[i] == '\'' {
                return Ok(i + 1);
            }
            i += 1;
        }
        Err(format!(
            "unterminated single quote opened on line {}",
            self.line(opened)
        ))
    }

    /// `$'...'` starting after the opening quote (backslash escapes active).
    fn ansi_c(&self, mut i: usize, opened: usize) -> Lx {
        while i < self.chars.len() {
            match self.chars[i] {
                '\\' => i += 2,
                '\'' => return Ok(i + 1),
                _ => i += 1,
            }
        }
        Err(format!(
            "unterminated $'...' opened on line {}",
            self.line(opened)
        ))
    }

    /// `"..."` starting after the opening quote; substitutions inside are lexed.
    fn double(&mut self, i: usize, opened: usize) -> Lx {
        self.nested(opened, |s| s.double_body(i, opened))
    }

    fn double_body(&mut self, mut i: usize, opened: usize) -> Lx {
        while i < self.chars.len() {
            let next = self.chars.get(i + 1).copied();
            match self.chars[i] {
                '"' => return Ok(i + 1),
                '\\' => i += 2,
                '$' if next == Some('(') => i = self.dollar_paren(i)?,
                '$' if next == Some('{') => i = self.brace(i + 2, i)?,
                '`' => i = self.list(i + 1, true, Term::Backtick)?,
                _ => i += 1,
            }
        }
        Err(format!(
            "unterminated double quote opened on line {}",
            self.line(opened)
        ))
    }

    /// `${...}` starting after the opening brace (quotes and substitutions inside followed,
    /// as SET41's scanner does).
    fn brace(&mut self, i: usize, opened: usize) -> Lx {
        self.nested(opened, |s| s.brace_body(i, opened))
    }

    fn brace_body(&mut self, mut i: usize, opened: usize) -> Lx {
        while i < self.chars.len() {
            let next = self.chars.get(i + 1).copied();
            match self.chars[i] {
                '}' => return Ok(i + 1),
                '\\' => i += 2,
                '\'' => i = self.single(i + 1, i)?,
                '"' => i = self.double(i + 1, i)?,
                '$' if next == Some('(') => i = self.dollar_paren(i)?,
                '$' if next == Some('{') => i = self.brace(i + 2, i)?,
                '`' => i = self.list(i + 1, true, Term::Backtick)?,
                _ => i += 1,
            }
        }
        Err(format!(
            "unterminated ${{ opened on line {}",
            self.line(opened)
        ))
    }

    /// `$(` at `i`: arithmetic `$(( ... ))` when it reads as one, else a command substitution;
    /// returns the index after its end.
    fn dollar_paren(&mut self, i: usize) -> Lx {
        if self.chars.get(i + 2) == Some(&'(') {
            if let Some(end) = self.try_arith(i + 3)? {
                return Ok(end);
            }
        }
        self.list(i + 2, true, Term::Paren)
    }

    /// Arithmetic after `((` at `start`: `Some(index after the closing "))")`, or `None` when
    /// the text is not arithmetic (`((a) | b)` is two subshells), in which case every command
    /// read meanwhile is dropped so the caller can read it again as commands.
    fn try_arith(&mut self, start: usize) -> Result<Option<usize>, String> {
        if self.not_arith.contains(&start) {
            return Ok(None);
        }
        let (cmds, pending) = (self.cmds.len(), self.pending.len());
        let found = self.nested(start, |s| s.arith_body(start))?;
        if found.is_none() {
            self.cmds.truncate(cmds);
            self.pending.truncate(pending);
            self.not_arith.insert(start);
        }
        Ok(found)
    }

    fn arith_body(&mut self, mut i: usize) -> Result<Option<usize>, String> {
        let mut depth = 0usize;
        while i < self.chars.len() {
            let next = self.chars.get(i + 1).copied();
            match self.chars[i] {
                '(' => {
                    depth += 1;
                    i += 1;
                }
                ')' if depth > 0 => {
                    depth -= 1;
                    i += 1;
                }
                ')' => return Ok((next == Some(')')).then_some(i + 2)),
                '\\' => i += 2,
                '\'' => i = self.single(i + 1, i)?,
                '"' => i = self.double(i + 1, i)?,
                '`' => i = self.list(i + 1, true, Term::Backtick)?,
                '$' if next == Some('(') => i = self.dollar_paren(i)?,
                '$' if next == Some('{') => i = self.brace(i + 2, i)?,
                _ => i += 1,
            }
        }
        Ok(None)
    }

    /// Bodies of the here-documents opened on the line that just ended, starting at `i`.
    fn heredoc_bodies(&mut self, mut i: usize) -> Lx {
        for (delim, expand) in std::mem::take(&mut self.pending) {
            let opened = i;
            loop {
                if i >= self.chars.len() {
                    return Err(format!(
                        "unterminated here-document {delim} (body from line {})",
                        self.line(opened)
                    ));
                }
                let mut end = i;
                while end < self.chars.len() && self.chars[end] != '\n' {
                    end += 1;
                }
                let text: String = self.chars[i..end].iter().collect();
                if text.trim_start_matches('\t') == delim {
                    i = (end + 1).min(self.chars.len());
                    break;
                }
                let mut j = i;
                while expand && j < end {
                    let next = self.chars.get(j + 1).copied();
                    match self.chars[j] {
                        '\\' => j += 2,
                        '$' if next == Some('(') => j = self.dollar_paren(j)?,
                        '$' if next == Some('{') => j = self.brace(j + 2, j)?,
                        '`' => j = self.list(j + 1, true, Term::Backtick)?,
                        _ => j += 1,
                    }
                }
                i = (j.max(end) + 1).min(self.chars.len());
            }
        }
        Ok(i)
    }

    /// A list of commands from `i` up to `term`; returns the index after the terminator.
    fn list(&mut self, i: usize, in_subst: bool, term: Term) -> Lx {
        self.nested(i, |s| s.list_body(i, in_subst, term))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "One match over the shell metacharacters reads best as a whole"
    )]
    fn list_body(&mut self, mut i: usize, in_subst: bool, term: Term) -> Lx {
        let opened = i;
        let mut acc = Acc::default();
        // `( ... )` subshells and `(( ... ))` opened inside this list.
        let mut paren = 0usize;
        // An open `NAME=(` array value (kept as one word).
        let mut array = false;
        // Literal parentheses inside a word (`name()`, extglob, `[[ =~ ]]` groups).
        let mut lit_paren = 0usize;
        loop {
            if i >= self.chars.len() {
                if term != Term::Top {
                    return Err(format!(
                        "unterminated {term:?} opened on line {}",
                        self.line(opened)
                    ));
                }
                self.end_cmd(&mut acc, in_subst);
                if paren != 0 || array || lit_paren != 0 || acc.case_depth != 0 || acc.test {
                    return Err(format!(
                        "the script does not end balanced (paren {paren}, array {array}, \
                         literal paren {lit_paren}, open case {}, open [[ {})",
                        acc.case_depth, acc.test
                    ));
                }
                if !self.pending.is_empty() {
                    return Err(format!("here-document never started: {:?}", self.pending));
                }
                return Ok(i);
            }
            let ch = self.chars[i];
            let next = self.chars.get(i + 1).copied();
            if ch == '(' && next == Some('(') && !acc.started && !array && !acc.pattern && !acc.test
            {
                if let Some(end) = self.try_arith(i + 2)? {
                    let text = self.raw(i, end);
                    acc.push(i, &text);
                    acc.end_word();
                    i = end;
                    continue;
                }
            }
            match ch {
                '\\' if next == Some('\n') => i += 2,
                '\\' => {
                    let end = (i + 2).min(self.chars.len());
                    let text = self.raw(i, end);
                    acc.push(i, &text);
                    i = end;
                }
                ' ' | '\t' if array => {
                    acc.push(i, " ");
                    i += 1;
                }
                ' ' | '\t' => {
                    acc.end_word();
                    i += 1;
                }
                '\n' if array => {
                    acc.push(i, " ");
                    i += 1;
                }
                '\n' if acc.in_test() => {
                    acc.end_word();
                    i += 1;
                }
                '\n' => {
                    self.end_cmd(&mut acc, in_subst);
                    i = self.heredoc_bodies(i + 1)?;
                }
                '#' if !acc.started => {
                    while i < self.chars.len() && self.chars[i] != '\n' {
                        i += 1;
                    }
                }
                '&' if next == Some('>') => {
                    acc.push(i, "&>");
                    i += 2;
                }
                ';' if acc.in_test() => {
                    return Err(format!(
                        "`;` inside an open [[ ... ]] test (line {})",
                        self.line(i)
                    ));
                }
                ';' if acc.case_depth > 0 && matches!(next, Some(';' | '&')) => {
                    // `;;`, `;&` or `;;&`: the next words are a pattern list.
                    self.end_cmd(&mut acc, in_subst);
                    acc.pattern = true;
                    i += 2;
                    if next == Some(';') && self.chars.get(i) == Some(&'&') {
                        i += 1;
                    }
                }
                '&' | '|' if acc.in_test() => {
                    // `&&` and `||` inside `[[ ... ]]` join tests, not commands.
                    acc.end_word();
                    let len = if next == Some(ch) { 2 } else { 1 };
                    let text = self.raw(i, i + len);
                    acc.push(i, &text);
                    acc.end_word();
                    i += len;
                }
                ';' | '&' | '|' => {
                    self.end_cmd(&mut acc, in_subst);
                    i += 1;
                }
                '<' | '>' if next == Some('(') => {
                    let start = i;
                    i = self.list(i + 2, true, Term::Paren)?;
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '<' if next == Some('<') && self.chars.get(i + 2) == Some(&'<') => {
                    acc.end_word();
                    i += 3;
                }
                '<' if next == Some('<') => {
                    acc.end_word();
                    let mut j = i + 2;
                    if self.chars.get(j) == Some(&'-') {
                        j += 1;
                    }
                    while self.chars.get(j) == Some(&' ') {
                        j += 1;
                    }
                    let quote = self
                        .chars
                        .get(j)
                        .copied()
                        .filter(|c| matches!(c, '\'' | '"'));
                    let start = if quote.is_some() { j + 1 } else { j };
                    let mut end = start;
                    while end < self.chars.len()
                        && (self.chars[end].is_ascii_alphanumeric() || self.chars[end] == '_')
                    {
                        end += 1;
                    }
                    let delim: String = self.chars[start.min(end)..end].iter().collect();
                    if delim.is_empty() {
                        return Err(format!(
                            "here-document without a delimiter (line {})",
                            self.line(i)
                        ));
                    }
                    if quote.is_some() && self.chars.get(end).copied() != quote {
                        return Err(format!(
                            "here-document delimiter {delim} quote not closed (line {})",
                            self.line(i)
                        ));
                    }
                    self.pending.push((delim, quote.is_none()));
                    i = if quote.is_some() { end + 1 } else { end };
                }
                '<' | '>' => {
                    let text = if next == Some('&') { "&" } else { "" };
                    acc.push(i, &format!("{ch}{text}"));
                    i += 1 + text.len();
                }
                '(' if acc.pattern && !acc.started => {
                    // The optional `(` before a `case` pattern.
                    i += 1;
                }
                '(' if array => {
                    acc.push(i, "(");
                    i += 1;
                }
                '(' if acc.started && acc.word.ends_with('=') => {
                    array = true;
                    acc.push(i, "(");
                    i += 1;
                }
                '(' if acc.test && !acc.started => {
                    // A grouping inside `[[ ... ]]`.
                    lit_paren += 1;
                    acc.push(i, "(");
                    i += 1;
                }
                '(' if !acc.started => {
                    self.end_cmd(&mut acc, in_subst);
                    paren += 1;
                    i += 1;
                }
                '(' => {
                    lit_paren += 1;
                    acc.push(i, "(");
                    i += 1;
                }
                ')' if array => {
                    acc.push(i, ")");
                    array = false;
                    i += 1;
                }
                ')' if lit_paren > 0 => {
                    lit_paren -= 1;
                    acc.push(i, ")");
                    i += 1;
                }
                ')' if acc.pattern && !acc.at_esac() => {
                    // The end of a `case` pattern list.
                    acc.push(i, ")");
                    self.end_cmd(&mut acc, in_subst);
                    acc.pattern = false;
                    i += 1;
                }
                ')' if paren > 0 => {
                    self.end_cmd(&mut acc, in_subst);
                    paren -= 1;
                    i += 1;
                }
                ')' if term == Term::Paren => {
                    self.end_cmd(&mut acc, in_subst);
                    if acc.case_depth > 0 || acc.test {
                        return Err(format!(
                            "case without esac or [[ without ]] in the substitution opened on \
                             line {}",
                            self.line(opened)
                        ));
                    }
                    return Ok(i + 1);
                }
                ')' => {
                    // A `case` pattern label.
                    acc.push(i, ")");
                    self.end_cmd(&mut acc, in_subst);
                    i += 1;
                }
                '`' if term == Term::Backtick => {
                    self.end_cmd(&mut acc, in_subst);
                    if acc.case_depth > 0 || acc.test {
                        return Err(format!(
                            "case without esac or [[ without ]] in the backticks opened on \
                             line {}",
                            self.line(opened)
                        ));
                    }
                    return Ok(i + 1);
                }
                '`' => {
                    let start = i;
                    i = self.list(i + 1, true, Term::Backtick)?;
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '$' if next == Some('(') => {
                    let start = i;
                    i = self.dollar_paren(i)?;
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '$' if next == Some('{') => {
                    let start = i;
                    i = self.brace(i + 2, i)?;
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '$' if next == Some('\'') => {
                    let start = i;
                    i = self.ansi_c(i + 2, i)?;
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '\'' => {
                    let start = i;
                    i = self.single(i + 1, i)?;
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '"' => {
                    let start = i;
                    i = self.double(i + 1, i)?;
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                _ => {
                    acc.push(i, &ch.to_string());
                    i += 1;
                }
            }
        }
    }
}

/// Every simple command of `script`, in substitutions too (a substitution's commands are
/// listed on their own, and its raw text also stays in the enclosing word), or why the script
/// cannot be read (unbalanced, truncated or nested deeper than `MAX_NESTING`). Never panics.
fn try_lex(script: &str) -> Result<Vec<ShCmd>, String> {
    let chars: Vec<char> = script.chars().collect();
    let mut line_of = Vec::with_capacity(chars.len());
    let mut line = 1;
    for c in &chars {
        line_of.push(line);
        if *c == '\n' {
            line += 1;
        }
    }
    let mut lexer = Lexer {
        chars: &chars,
        line_of,
        cmds: Vec::new(),
        pending: Vec::new(),
        depth: 0,
        not_arith: std::collections::HashSet::new(),
    };
    let end = lexer.list(0, false, Term::Top)?;
    if end != chars.len() {
        return Err(format!(
            "the lexer stopped at {end} before the end ({})",
            chars.len()
        ));
    }
    Ok(lexer.cmds)
}

/// `try_lex` for the checks: a script the lexer cannot follow fails the test loudly, with the
/// reason, instead of passing.
fn lex(script: &str) -> Vec<ShCmd> {
    try_lex(script).unwrap_or_else(|e| panic!("setup.sh lexer: {e}"))
}

/// A word without its quotes and backslashes.
fn unquote(word: &str) -> String {
    word.chars().filter(|c| !"\"'\\".contains(*c)).collect()
}

// ---------------------------------------------------------------------------
// Tailscale checks
// ---------------------------------------------------------------------------

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_.-".contains(c)
}

/// Byte offsets of every `tailscale` command word in `text`: bare, quoted or as the last
/// component of a path (`/usr/bin/tailscale`), never part of a longer name
/// (`tailscale_version`, `--tailscale-login`, `tailscale.com`) or a directory (`tailscale/`).
fn tailscale_tokens(text: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(pos) = text[from..].find("tailscale").map(|p| p + from) {
        let before = text[..pos].chars().next_back();
        let after = text[pos + "tailscale".len()..].chars().next();
        if !before.is_some_and(is_name_char) && !after.is_some_and(|c| is_name_char(c) || c == '/')
        {
            found.push(pos);
        }
        from = pos + "tailscale".len();
    }
    found
}

/// Whether the `tailscale` word at `pos` is followed, past quotes and at least one blank, by
/// the word `version`.
fn followed_by_version(text: &str, pos: usize) -> bool {
    let rest = &text[pos + "tailscale".len()..];
    let gap: String = rest.chars().take_while(|c| "\"' \t".contains(*c)).collect();
    let rest = &rest[gap.len()..];
    gap.contains([' ', '\t'])
        && rest.starts_with("version")
        && !rest["version".len()..]
            .chars()
            .next()
            .is_some_and(is_name_char)
}

/// Whether `text`, as written or after shell quote removal (`"tail""scale"`, `\tailscale`,
/// `'tail''scale'`, `$'\x74ailscale'`), has a `tailscale` word other than `tailscale version`.
fn tailscale_commands(text: &str) -> bool {
    let any = |text: &str| {
        tailscale_tokens(text)
            .into_iter()
            .any(|pos| !followed_by_version(text, pos))
    };
    any(text) || any(&quote_removed(text))
}

/// `text` with its quotes and backslashes removed and its `$'...'` escapes decoded, so a
/// name spelled through quotes or escapes reads as the shell runs it. Only used to look for
/// more Tailscale words (the text as written is always checked too).
fn quote_removed(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '$' if chars.get(i + 1) == Some(&'\'') => {
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        let (decoded, used) = ansi_c_escape(&chars[i + 1..]);
                        out.push_str(&decoded);
                        i += 1 + used;
                    } else {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
                i += 1;
            }
            '\'' | '"' | '\\' => i += 1,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// The text of the `$'...'` escape whose letter starts `rest` (after the backslash), and how
/// many chars of `rest` it uses.
fn ansi_c_escape(rest: &[char]) -> (String, usize) {
    let digits = |radix: u32, skip: usize, max: usize| {
        let taken: String = rest
            .iter()
            .skip(skip)
            .take(max)
            .take_while(|c| c.is_digit(radix))
            .collect();
        let value = u32::from_str_radix(&taken, radix).ok();
        let decoded = value.and_then(char::from_u32).map(String::from);
        (decoded, skip + taken.chars().count())
    };
    let (decoded, used) = match rest.first().copied() {
        Some('x') => digits(16, 1, 2),
        Some('u') => digits(16, 1, 4),
        Some('U') => digits(16, 1, 8),
        Some('0'..='7') => digits(8, 0, 3),
        Some('n') => (Some("\n".to_string()), 1),
        Some('t') => (Some("\t".to_string()), 1),
        Some(c) => (Some(c.to_string()), 1),
        None => (None, 0),
    };
    (decoded.unwrap_or_default(), used.max(1).min(rest.len()))
}

/// SCN2 scanner: a Tailscale word other than `tailscale version` inside a command or process
/// substitution, by bare name, quoted name or absolute path, continuation lines joined.
fn tailscale_in_substitution_by_any_name(script: &str) -> Vec<String> {
    lex(script)
        .into_iter()
        .filter(|cmd| cmd.in_subst)
        .filter_map(|cmd| {
            let text = cmd.words.join(" ");
            tailscale_commands(&text).then(|| format!("line {}: {text}", cmd.line))
        })
        .collect()
}

/// Length of the `NAME=`, `NAME+=` or `NAME[...]=` prefix of an assignment word.
fn assignment_prefix(word: &str) -> Option<usize> {
    let mut chars = word.char_indices().peekable();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return None,
    }
    while let Some(&(at, c)) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '_' {
            chars.next();
            continue;
        }
        let rest = &word[at..];
        let rest = if rest.starts_with('[') {
            &rest[rest.find(']')? + 1..]
        } else {
            rest
        };
        let offset = word.len() - rest.len();
        return if rest.starts_with("+=") {
            Some(offset + 2)
        } else if rest.starts_with('=') {
            Some(offset + 1)
        } else {
            None
        };
    }
    None
}

/// `text` without its `${...}` parameter expansions: a pattern such as `${x#tailscale }` names
/// no command, and the fallback operands are checked on their own by `parameter_defaults`.
fn without_parameter_expansions(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next();
            depth += 1;
        } else if depth > 0 && c == '{' {
            depth += 1;
        } else if depth > 0 && c == '}' {
            depth -= 1;
        } else if depth == 0 {
            out.push(c);
        }
    }
    out
}

/// Operands of the `${NAME:-x}`, `${NAME-x}`, `${NAME:=x}`, `${NAME=x}`, `${NAME:+x}` and
/// `${NAME+x}` expansions of `word` (the value a variable falls back to).
fn parameter_defaults(word: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(start) = word[from..].find("${").map(|p| p + from + 2) {
        from = start;
        let name_len = word[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .count();
        let op_at = start + name_len;
        let rest = &word[op_at..];
        let op_len = [":-", ":=", ":+", "-", "=", "+"]
            .iter()
            .find(|op| rest.starts_with(**op))
            .map(|op| op.len());
        let (Some(op_len), true) = (op_len, name_len > 0) else {
            continue;
        };
        let operand_at = op_at + op_len;
        let mut depth = 1usize;
        let mut end = word.len();
        for (offset, c) in word[operand_at..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = operand_at + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        found.push(word[operand_at..end].to_string());
    }
    found
}

/// SCN3 scanner: a Tailscale command (any word but `tailscale version`) held in a variable:
/// an assignment (`NAME=`, `NAME+=`, `NAME[i]=`, arrays, `local`/`declare`/`readonly`/
/// `export` arguments, `alias`), a parameter default (`${NAME:-tailscale}`), `printf -v` or
/// a `read`/`mapfile`/`readarray` command.
fn tailscale_held_in_variable(script: &str) -> Vec<String> {
    let mut found = Vec::new();
    for cmd in lex(script) {
        let text = cmd.words.join(" ");
        let mut bad = false;
        for word in &cmd.words {
            if let Some(len) = assignment_prefix(word) {
                bad |= tailscale_commands(&without_parameter_expansions(&word[len..]));
            }
            for operand in parameter_defaults(word) {
                bad |= tailscale_commands(&operand);
            }
        }
        for (k, word) in cmd.words.iter().enumerate() {
            let rest = cmd.words[k + 1..].join(" ");
            match unquote(word).as_str() {
                "printf" if cmd.words.get(k + 1).map(String::as_str) == Some("-v") => {
                    bad |= tailscale_commands(&rest);
                }
                "read" | "mapfile" | "readarray" => bad |= tailscale_commands(&rest),
                _ => {}
            }
        }
        if bad {
            found.push(format!("line {}: {text}", cmd.line));
        }
    }
    found
}

// ---------------------------------------------------------------------------
// Daemon unit checks
// ---------------------------------------------------------------------------

/// `systemctl` verbs that start, stop or restart a unit.
const MUTATING_VERBS: [&str; 6] = [
    "start",
    "restart",
    "stop",
    "try-restart",
    "reload-or-restart",
    "try-reload-or-restart",
];

/// `systemctl` verbs that only read the state of a unit.
const QUERY_VERBS: [&str; 3] = ["is-enabled", "is-active", "is-failed"];

/// The names under which setup.sh may designate the daemon unit.
const DAEMON_UNIT_NAMES: [&str; 4] = ["${UNIT}", "$UNIT", "soos-daemon", "soos-daemon.service"];

/// SCN4 scanner: returns the violations and the number of correctly bounded daemon calls.
///
/// After continuation lines are joined, every `systemctl` (bare or by path) call of the
/// system daemon unit whose verb starts, stops or restarts it (`enable`/`disable --now`
/// included) is immediately preceded by `run_bounded "${UNIT_START_TIMEOUT_S}"`; a call with a
/// read-only verb is bounded by `QUERY_TIMEOUT_S`; a verb held in a positional parameter is
/// only the `unit_query` helper's `run_bounded "${QUERY_TIMEOUT_S}" systemctl "$1" --quiet
/// "${UNIT}"`, and every `unit_query` call passes a read-only verb; `UNIT_START_TIMEOUT_S`
/// never bounds a `systemctl --user` call.
fn daemon_unit_call_violations(script: &str) -> (Vec<String>, usize) {
    let mut found = Vec::new();
    let mut bounded = 0;
    for cmd in lex(script) {
        let w = &cmd.words;
        let text = w.join(" ");
        let at = |k: usize| w.get(k).map(String::as_str).unwrap_or("");
        for (k, word) in w.iter().enumerate() {
            let plain = unquote(word);
            if plain == "unit_query" && !QUERY_VERBS.contains(&unquote(at(k + 1)).as_str()) {
                found.push(format!(
                    "line {}: unit_query only takes a read-only verb: {text}",
                    cmd.line
                ));
            }
            if plain.rsplit('/').next() != Some("systemctl") {
                continue;
            }
            let args: Vec<String> = w[k + 1..].iter().map(|a| unquote(a)).collect();
            let user = args.iter().any(|a| a == "--user");
            let now = args.iter().any(|a| a == "--now");
            let verb = args
                .iter()
                .find(|a| !a.starts_with('-'))
                .cloned()
                .unwrap_or_default();
            let bound = if k >= 2 && at(k - 2) == "run_bounded" {
                at(k - 1)
            } else {
                ""
            };
            if user {
                if bound.contains("UNIT_START_TIMEOUT_S") {
                    found.push(format!(
                        "line {}: UNIT_START_TIMEOUT_S bounds a systemctl --user call: {text}",
                        cmd.line
                    ));
                }
                continue;
            }
            let daemon = w[k + 1..]
                .iter()
                .any(|a| DAEMON_UNIT_NAMES.contains(&unquote(a).as_str()));
            if !daemon {
                continue;
            }
            let mutating = MUTATING_VERBS.contains(&verb.as_str())
                || (now && (verb == "enable" || verb == "disable"));
            if mutating {
                if bound == "\"${UNIT_START_TIMEOUT_S}\"" {
                    bounded += 1;
                } else {
                    found.push(format!(
                        "line {}: systemctl {verb} of the daemon unit is not bounded by \
                         UNIT_START_TIMEOUT_S: {text}",
                        cmd.line
                    ));
                }
            } else if QUERY_VERBS.contains(&verb.as_str()) {
                if bound != "\"${QUERY_TIMEOUT_S}\"" {
                    found.push(format!(
                        "line {}: a read-only daemon unit query is bounded by QUERY_TIMEOUT_S: \
                         {text}",
                        cmd.line
                    ));
                }
            } else if verb.starts_with('$') {
                let helper = [
                    "run_bounded",
                    "\"${QUERY_TIMEOUT_S}\"",
                    "systemctl",
                    "\"$1\"",
                    "--quiet",
                    "\"${UNIT}\"",
                ];
                if k < 2 || w[k - 2..].len() < helper.len() || w[k - 2..k + 4] != helper {
                    found.push(format!(
                        "line {}: a systemctl verb held in a variable is only the unit_query \
                         helper: {text}",
                        cmd.line
                    ));
                }
            } else {
                found.push(format!(
                    "line {}: unexpected systemctl {verb} of the daemon unit: {text}",
                    cmd.line
                ));
            }
        }
    }
    (found, bounded)
}

/// Every assignment of `UNIT` in `script` (assignment words, `for UNIT in`, `read ... UNIT`,
/// a bare `local`/`declare`/`typeset UNIT` that shadows it, `unset UNIT`).
fn unit_assignments(script: &str) -> Vec<String> {
    let mut found = Vec::new();
    for cmd in lex(script) {
        for (k, word) in cmd.words.iter().enumerate() {
            let assigned = assignment_prefix(word)
                .is_some_and(|len| word[..len].trim_end_matches(['=', '+']) == "UNIT");
            let loop_or_read = word == "UNIT"
                && k > 0
                && cmd.words[..k].iter().any(|w| {
                    [
                        "for",
                        "read",
                        "printf",
                        "mapfile",
                        "readarray",
                        "local",
                        "declare",
                        "typeset",
                        "unset",
                    ]
                    .contains(&w.as_str())
                });
            if assigned || loop_or_read {
                found.push(format!("line {}: {word}", cmd.line));
            }
        }
    }
    found
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn words(script: &str) -> Vec<Vec<String>> {
    lex(script).into_iter().map(|c| c.words).collect()
}

/// SCN1: the lexer joins backslash continuations into one simple command, keeps quoted
/// messages as one word, lists the commands of `$(...)`, backticks, `<(...)` and unquoted
/// here-documents as substitution commands (never those of a quoted here-document or a
/// comment), and an unbalanced script fails loudly instead of passing.
#[test]
fn test_scn1_lexer_joins_continuations_and_follows_contexts() {
    assert_eq!(
        words("run_bounded \"${T}\" \\\n    systemctl \\\n  start \"${UNIT}\" || rc=$?\n"),
        vec![
            vec![
                "run_bounded",
                "\"${T}\"",
                "systemctl",
                "start",
                "\"${UNIT}\""
            ],
            vec!["rc=$?"],
        ]
    );
    assert_eq!(
        words("run_bounded 5 tail\\\nscale serve\n"),
        vec![vec!["run_bounded", "5", "tailscale", "serve"]]
    );
    assert_eq!(
        words("error \"Could not start: sudo systemctl start soos-daemon\" # systemctl stop x\n"),
        vec![vec![
            "error",
            "\"Could not start: sudo systemctl start soos-daemon\""
        ]]
    );
    let cmds = lex("x=\"$(a 1 \"$(b 2)\")\"; y=`c 3`\ncmp -s f <(d 4)\n");
    let nested: Vec<(String, bool)> = cmds
        .iter()
        .map(|c| (c.words[0].clone(), c.in_subst))
        .collect();
    for (name, inside) in [
        ("b", true),
        ("a", true),
        ("x=\"$(a 1 \"$(b 2)\")\"", false),
        ("c", true),
        ("y=`c 3`", false),
        ("d", true),
        ("cmp", false),
    ] {
        assert!(
            nested.contains(&(name.to_string(), inside)),
            "{name} in_subst={inside}: {nested:?}"
        );
    }
    let here = lex("cat <<EOF\nnode: $(e 5)\nEOF\ncat <<'EOF'\nprinted: $(f 6)\nEOF\nok\n");
    assert!(here.iter().any(|c| c.words[0] == "e" && c.in_subst));
    assert!(!here.iter().any(|c| c.words[0] == "f"));
    assert_eq!(here.last().map(|c| c.line), Some(7));
    let regex = words("[[ \"${l}\" =~ ^[a-z]([a-z-]{0,61}[a-z])?$ ]] || return 1\n");
    assert_eq!(regex.len(), 2, "{regex:?}");
    let array = words("CMD=(run_bounded 5\n  g serve)\n\"${CMD[@]}\"\n");
    assert_eq!(array[0], vec!["CMD=(run_bounded 5   g serve)"]);
    for unbalanced in [
        "x=\"$(a\n",
        "x='a\n",
        "cat <<EOF\nbody\n",
        "y=`b\n",
        "f() { (a\n",
    ] {
        let result = std::panic::catch_unwind(|| lex(unbalanced));
        assert!(result.is_err(), "{unbalanced:?} must fail loudly");
    }
}

/// SCN2: no Tailscale command other than `tailscale version` inside a command or process
/// substitution by absolute path (`$(/usr/bin/tailscale serve)`), quoted name
/// (`$("tailscale" serve)`) or across a continuation line (the gaps of SET41), checked on
/// synthetic input first, then on setup.sh, where the read-only `tailscale version` call must
/// be seen.
#[test]
fn test_scn2_no_tailscale_in_substitution_by_path_or_quoted_name() {
    for (snippet, bad) in [
        ("x=\"$(/usr/bin/tailscale serve --bg unix:/run/x)\"\n", true),
        ("x=\"$(\"/usr/bin/tailscale\" funnel reset)\"\n", true),
        ("x=\"$(\"tailscale\" serve reset)\"\n", true),
        ("x=$('tailscale' up)\n", true),
        (
            "x=\"$(run_bounded 5 \\\n    /usr/bin/tailscale serve reset)\"\n",
            true,
        ),
        ("x=\"$(run_bounded 5 tail\\\nscale serve)\"\n", true),
        ("x=`/usr/local/bin/tailscale up`\n", true),
        ("cat <<EOF\nnode: $(/usr/bin/tailscale status)\nEOF\n", true),
        ("cmp -s a <(/usr/bin/tailscale serve status)\n", true),
        ("x=\"$(command -v tailscale)\"\n", true),
        (
            "out=\"$(run_bounded \"${QUERY_TIMEOUT_S}\" tailscale version 2>/dev/null)\"\n",
            false,
        ),
        ("out=\"$(/usr/bin/tailscale version)\"\n", false),
        ("out=\"$(\"tailscale\" version)\"\n", false),
        (
            "info \"       tailscale funnel --bg unix:${REMOTE_SOCKET}\"\n",
            false,
        ),
        (
            "x=\"$(printf '%s' https://tailscale.com/download/linux)\"\n",
            false,
        ),
        ("x=\"$(ls /var/lib/tailscale/)\"\n", false),
        ("x=\"$(tailscale_version)\"\n", false),
        (
            "cat <<'EOF'\nprinted: $(/usr/bin/tailscale serve)\nEOF\n",
            false,
        ),
        ("# x=\"$(/usr/bin/tailscale serve)\"\n", false),
    ] {
        let found = tailscale_in_substitution_by_any_name(snippet);
        assert_eq!(
            !found.is_empty(),
            bad,
            "scanner verdict for {snippet:?} must be {}: {found:?}",
            if bad { "rejected" } else { "accepted" }
        );
    }
    let script = read_repo(SETUP);
    let found = tailscale_in_substitution_by_any_name(&script);
    assert!(
        found.is_empty(),
        "setup.sh runs a Tailscale command other than `tailscale version` inside a \
         substitution: {found:?}"
    );
    assert!(
        lex(&script)
            .iter()
            .any(|c| c.in_subst && c.words.windows(2).any(|p| p == ["tailscale", "version"])),
        "the lexer must reach the read-only `tailscale version` substitution of setup.sh"
    );
}

/// SCN3: no Tailscale command is held in a variable (an assignment, an array, a parameter
/// default, `printf -v`, `read`/`mapfile`), the gap of SET3 and SET41, which only see a
/// literal `tailscale ` command word; checked on synthetic input first, then on setup.sh.
#[test]
fn test_scn3_no_tailscale_command_held_in_a_variable() {
    for (snippet, bad) in [
        ("TS=tailscale\n\"${TS}\" serve --bg unix:/x\n", true),
        ("local ts_bin=\"/usr/bin/tailscale\"\n", true),
        ("readonly TS='tailscale'\n", true),
        ("TS=\"$(command -v tailscale)\"\n", true),
        (
            "CMD=(run_bounded 5 tailscale funnel reset)\n\"${CMD[@]}\"\n",
            true,
        ),
        ("CMD+=(\n    tailscale serve\n)\n", true),
        ("CMD[2]=tailscale\n", true),
        ("export TS_CMD=\"tailscale serve --bg\"\n", true),
        ("declare -a T=(\"/usr/bin/tailscale\" up)\n", true),
        ("alias ts=tailscale\n", true),
        ("\"${TAILSCALE_BIN:-tailscale}\" serve reset\n", true),
        ("\"${TS:=/usr/bin/tailscale}\" up\n", true),
        ("printf -v TS '%s' tailscale\n", true),
        ("read -r TS <<< \"/usr/bin/tailscale\"\n", true),
        ("TS=tail\\\nscale\n", true),
        ("TAILSCALE_VERSION=\"$(tailscale_version)\"\n", false),
        (
            "out=\"$(run_bounded 10 tailscale version 2>/dev/null)\" || out=\"\"\n",
            false,
        ),
        ("v=${x#tailscale }\n", false),
        (
            "case \"$1\" in\n  --tailscale-login) TAILSCALE_LOGIN=\"$2\" ;;\nesac\n",
            false,
        ),
        (
            "plan tailscale \"printed only\" \"no tailscale serve or funnel\"\n",
            false,
        ),
        ("msg=\"see https://tailscale.com/download/linux\"\n", false),
        ("command -v tailscale >/dev/null 2>&1\n", false),
        ("read -r -a w <<< \"${x}\"\n", false),
    ] {
        let found = tailscale_held_in_variable(snippet);
        assert_eq!(
            !found.is_empty(),
            bad,
            "scanner verdict for {snippet:?} must be {}: {found:?}",
            if bad { "rejected" } else { "accepted" }
        );
    }
    let found = tailscale_held_in_variable(&read_repo(SETUP));
    assert!(
        found.is_empty(),
        "setup.sh holds a Tailscale command in a variable: {found:?}"
    );
}

/// SCN4: with continuation lines joined (the gap of SET40), every start, stop or restart of
/// the daemon unit (bare or absolute `systemctl`, in a substitution too, `enable --now`
/// included) is bounded by `UNIT_START_TIMEOUT_S`, read-only queries by `QUERY_TIMEOUT_S`, a
/// verb in `$1` only through the `unit_query` helper fed read-only verbs, and
/// `UNIT_START_TIMEOUT_S` never bounds a `systemctl --user` call; `UNIT` is assigned once, to
/// `soos-daemon.service`. Checked on synthetic input first, then on setup.sh, where the lexer
/// must count at least every call SET40's line check counts.
#[test]
fn test_scn4_daemon_unit_bound_survives_continuation_lines() {
    for (snippet, bad) in [
        (
            "run_bounded \"${QUERY_TIMEOUT_S}\" \\\n    systemctl start \"${UNIT}\"\n",
            true,
        ),
        ("systemctl \\\n    stop \"${UNIT}\"\n", true),
        ("run_bounded \"${UNIT_START_TIMEOUT_S}\" \\\n  true; systemctl start \"${UNIT}\"\n", true),
        ("if ! /usr/bin/systemctl try-restart \"${UNIT}\"; then :; fi\n", true),
        ("run_bounded \"${QUERY_TIMEOUT_S}\" systemctl  start  soos-daemon\n", true),
        ("x=\"$(systemctl restart \"${UNIT}\" 2>&1)\"\n", true),
        ("systemctl enable --now \"${UNIT}\"\n", true),
        ("run_bounded 5 systemctl reload-or-restart soos-daemon.service\n", true),
        ("systemctl --no-block start \"$UNIT\"\n", true),
        (
            "run_bounded \"${UNIT_START_TIMEOUT_S}\" systemctl --user restart soos-remote.service\n",
            true,
        ),
        ("systemctl is-active --quiet \"${UNIT}\"\n", true),
        ("systemctl \"${verb}\" \"${UNIT}\"\n", true),
        ("unit_query start\n", true),
        (
            "run_bounded \"${UNIT_START_TIMEOUT_S}\" systemctl \\\n  restart \\\n  \"${UNIT}\"\n",
            false,
        ),
        (
            "if run_bounded \"${UNIT_START_TIMEOUT_S}\" systemctl  start  soos-daemon.service; \
             then :; fi\n",
            false,
        ),
        (
            "run_bounded \"${QUERY_TIMEOUT_S}\" systemctl \"$1\" --quiet \"${UNIT}\" 2>/dev/null\n",
            false,
        ),
        (
            "printf 'sudo systemctl stop soos-daemon && sudo soos-enroll enroll' \\\n    \"${x}\"\n",
            false,
        ),
        (
            "error \"Could not start ${UNIT} again: sudo systemctl start soos-daemon\"\n",
            false,
        ),
        (
            "run_bounded \"${QUERY_TIMEOUT_S}\" systemctl --user try-restart soos-remote.service \\\n    || return\n",
            false,
        ),
        ("unit_query is-enabled || return 1\nunit_query is-active\n", false),
    ] {
        let (found, _) = daemon_unit_call_violations(snippet);
        assert_eq!(
            !found.is_empty(),
            bad,
            "scanner verdict for {snippet:?} must be {}: {found:?}",
            if bad { "rejected" } else { "accepted" }
        );
    }
    assert_eq!(
        unit_assignments("UNIT=\"soos-daemon.service\"\nf() { local UNIT=x; }\n").len(),
        2
    );
    assert_eq!(
        unit_assignments("for UNIT in a b; do :; done\nUNIT_X=1\n").len(),
        1
    );
    assert_eq!(
        unit_assignments("f() { local UNIT; }\nunset UNIT\necho UNIT\n").len(),
        2
    );

    let script = read_repo(SETUP);
    let (found, bounded) = daemon_unit_call_violations(&script);
    assert!(found.is_empty(), "setup.sh daemon unit calls: {found:?}");
    let per_line: usize = script
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| {
            ["start", "restart", "stop", "try-restart"]
                .iter()
                .filter(|verb| l.contains(&format!("systemctl {verb} \"${{UNIT}}\"")))
                .count()
        })
        .sum();
    assert!(
        bounded >= 6 && bounded >= per_line,
        "the lexer must see every bounded daemon unit call SET40 counts line by line \
         ({bounded} < {per_line}, or fewer than 6)"
    );
    let assignments = unit_assignments(&script);
    assert_eq!(
        assignments.len(),
        1,
        "UNIT is assigned exactly once: {assignments:?}"
    );
    assert!(
        script
            .lines()
            .any(|l| l.trim() == "UNIT=\"soos-daemon.service\""),
        "UNIT names the system daemon unit"
    );
}

/// SCN5: traceability: the matrix carries SCN1-SCN5 and walkthrough 245 exists; SET40 and
/// SET41 stay in `setup_contract.rs` next to these additive checks.
#[test]
fn test_scn5_traceability() {
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for row in 1..=5 {
        assert!(
            matrix.contains(&format!("| SCN{row} |")),
            "matrix row SCN{row}"
        );
    }
    assert!(workspace_root()
        .join("AI/walkthroughs/245_setup_scanners.md")
        .is_file());
    let contract = read_repo("tests/invariants/src/setup_contract.rs");
    for test in [
        "fn test_set_s6_daemon_unit_calls_bounded_by_unit_start_timeout()",
        "fn test_set_s7_no_tailscale_command_inside_command_substitution()",
    ] {
        assert!(contract.contains(test), "{test} is kept");
    }
}

// ---------------------------------------------------------------------------
// Command words taken from variables (SCN7)
// ---------------------------------------------------------------------------

/// A variable expanded by a word: `$NAME`, `${NAME...}`, `$1`, `$@`, `${!NAME}`.
#[derive(Debug, PartialEq, Eq)]
enum VarRef {
    Named(String),
    /// A positional or special parameter (`$1`, `$@`, `$*`, `$#`, `$?`, `$$`, `$!`, `$-`).
    Special(char),
    /// `${!NAME}`: the variable named by another one.
    Indirect(String),
}

/// The variables a raw word or value expands, inside its substitutions too (single-quoted
/// text excluded; `${#NAME}` is the length of `NAME`).
fn variable_refs(text: &str) -> Vec<VarRef> {
    let chars: Vec<char> = text.chars().collect();
    let mut refs = Vec::new();
    let mut in_double = false;
    let mut i = 0;
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    while i < chars.len() {
        let next = chars.get(i + 1).copied();
        match chars[i] {
            '\\' => i += 2,
            '"' => {
                in_double = !in_double;
                i += 1;
            }
            '\'' if !in_double => {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            '$' => {
                let braced = next == Some('{');
                let mut j = if braced { i + 2 } else { i + 1 };
                let indirect = braced && chars.get(j) == Some(&'!');
                let length = braced
                    && chars.get(j) == Some(&'#')
                    && chars
                        .get(j + 1)
                        .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_');
                if indirect || length {
                    j += 1;
                }
                let first = chars.get(j).copied();
                match first {
                    Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                        let start = j;
                        while j < chars.len() && is_name(chars[j]) {
                            j += 1;
                        }
                        let name: String = chars[start..j].iter().collect();
                        refs.push(if indirect {
                            VarRef::Indirect(name)
                        } else {
                            VarRef::Named(name)
                        });
                    }
                    Some(c) if c.is_ascii_digit() || "@*#?$!-".contains(c) => {
                        refs.push(VarRef::Special(c));
                        j += 1;
                    }
                    _ => {}
                }
                i = j.max(i + 1);
            }
            _ => i += 1,
        }
    }
    refs
}

/// Whether a command word or value names a fixed program by its last path component
/// (`"${PREFIX}/bin/soos-admin"`, `"${BIN_DIR}/soos-remote"`): a variable may choose the
/// directory, never which program runs.
/// Only when every expansion is double-quoted: an unquoted one is word split, so
/// `TS='tailscale serve '` makes `$TS/x` run `tailscale`.
fn names_fixed_program(text: &str) -> bool {
    if !all_expansions_quoted(text) {
        return false;
    }
    let plain = unquote(text);
    plain.rsplit_once('/').is_some_and(|(_, last)| {
        !last.is_empty()
            && last
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    })
}

/// Whether every `$` expansion and backtick substitution of a raw word sits inside double
/// quotes (`$'...'` and `$"..."` are quotes, not expansions).
fn all_expansions_quoted(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut in_double = false;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => {
                in_double = !in_double;
                i += 1;
            }
            '\'' if !in_double => {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            '$' if !in_double && matches!(chars.get(i + 1), Some('\'' | '"')) => i += 1,
            '$' | '`' if !in_double => return false,
            _ => i += 1,
        }
    }
    true
}

/// Where a variable gets its value in the script.
enum Origin {
    /// An assignment word's value, or a `for NAME in` list word.
    Value(String),
    /// `read`, `mapfile`, `readarray`, `printf -v`: from input, not from the script.
    Input,
}

/// Every variable the script assigns, with the origin of each of its values.
fn script_assignments(cmds: &[ShCmd]) -> std::collections::HashMap<String, Vec<Origin>> {
    let mut map: std::collections::HashMap<String, Vec<Origin>> = std::collections::HashMap::new();
    let is_ident = |w: &str| {
        w.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    for cmd in cmds {
        let w = &cmd.words;
        for (k, word) in w.iter().enumerate() {
            if let Some(len) = assignment_prefix(word) {
                let name = word[..len].trim_end_matches(['=', '+']);
                let name = name.split('[').next().unwrap_or(name).to_string();
                map.entry(name)
                    .or_default()
                    .push(Origin::Value(word[len..].to_string()));
                continue;
            }
            let before =
                |names: &[&str]| k > 0 && w[..k].iter().any(|p| names.contains(&p.as_str()));
            if !is_ident(word) {
                continue;
            }
            if k == 1 && w[0] == "for" {
                let list = w.iter().skip_while(|x| *x != "in").skip(1);
                let values: Vec<Origin> = list.map(|v| Origin::Value(v.clone())).collect();
                map.entry(word.clone()).or_default().extend(values);
            } else if before(&["read", "mapfile", "readarray"])
                || (k >= 2 && w[k - 1] == "-v" && w[k - 2] == "printf")
            {
                map.entry(word.clone()).or_default().push(Origin::Input);
            } else if before(&["local", "declare", "typeset", "readonly", "export"]) {
                map.entry(word.clone())
                    .or_default()
                    .push(Origin::Value(String::new()));
            }
        }
    }
    map
}

/// The index of the word a command runs, past assignment words, compound-command keywords,
/// function headers and the wrappers that run their arguments (`run_bounded <secs>`,
/// `as_root`, `as_build_user`, `command`, `exec`, `env`, `timeout`, `sudo`, `doas`, ...);
/// `None` when it runs none (assignments only, `command -v`).
fn command_word_index(words: &[String]) -> Option<usize> {
    let mut k = 0;
    loop {
        let word = words.get(k)?;
        if assignment_prefix(word).is_some() {
            k += 1;
            continue;
        }
        let plain = unquote(word);
        let option = |w: Option<&String>| w.is_some_and(|w| w.starts_with('-'));
        if plain.starts_with("((") {
            // An arithmetic command computes a number; it runs no program.
            return None;
        }
        match plain.as_str() {
            "if" | "then" | "elif" | "else" | "do" | "while" | "until" | "!" | "{" | "time"
            | "nohup" | "exec" | "builtin" | "eval" | "as_root" | "as_build_user" => k += 1,
            "command" => {
                if matches!(
                    words.get(k + 1).map(|w| unquote(w)).as_deref(),
                    Some("-v" | "-V")
                ) {
                    return None;
                }
                k += 1;
            }
            "run_bounded" => k += 2,
            "sudo" | "doas" | "env" | "timeout" | "xargs" | "nice" | "setsid" | "stdbuf" => {
                // Options that take their value as the next word.
                const WITH_VALUE: [&str; 11] = [
                    "-n", "-I", "-P", "-d", "-a", "-L", "-s", "-E", "-u", "-g", "-k",
                ];
                let env = plain == "env";
                k += 1;
                while option(words.get(k))
                    || (env && words.get(k).is_some_and(|w| assignment_prefix(w).is_some()))
                {
                    if WITH_VALUE.contains(&unquote(&words[k]).as_str()) {
                        k += 1;
                    }
                    k += 1;
                }
                if plain == "timeout" {
                    k += 1;
                }
            }
            _ if plain.ends_with("()") => k += 1,
            _ => return Some(k),
        }
    }
}

/// Why the variable `name` can choose a program the script did not write: it is never
/// assigned, it is read from input, one of its values expands itself, an indirect reference
/// or a variable with one of those problems (a value that names a fixed program is not
/// followed further when the use is `quoted_use`); `None` when every value is written by the
/// script.
fn inherited_reason(
    name: &str,
    quoted_use: bool,
    assigned: &std::collections::HashMap<String, Vec<Origin>>,
    stack: &mut Vec<String>,
) -> Option<String> {
    if stack.iter().any(|s| s == name) {
        return Some(format!("{name} is assigned from its own (inherited) value"));
    }
    let Some(origins) = assigned.get(name) else {
        return Some(format!(
            "{name} is never assigned by the script, so its value is inherited"
        ));
    };
    stack.push(name.to_string());
    let mut reason = None;
    for origin in origins {
        let value = match origin {
            Origin::Input => {
                reason = Some(format!("{name} is read from input"));
                break;
            }
            Origin::Value(value) => value,
        };
        if quoted_use && names_fixed_program(value) {
            continue;
        }
        for r in variable_refs(value) {
            reason = match r {
                VarRef::Named(dep) => inherited_reason(&dep, quoted_use, assigned, stack),
                VarRef::Indirect(dep) => Some(format!("{name} expands ${{!{dep}}}")),
                VarRef::Special(_) => None,
            };
            if reason.is_some() {
                break;
            }
        }
        if reason.is_some() {
            break;
        }
    }
    stack.pop();
    reason
}

/// SCN7 scanner: returns the violations and the variables seen in command words.
///
/// A command word built from a variable (`"${TS}"`, `$TS`, `"${X[@]}"`, `"${PREFIX}/bin/x"`)
/// only expands variables the script assigns from values it writes itself, transitively; a
/// positional parameter is a command word only as `"$@"`. Bare command names still resolve
/// through the inherited `PATH`, which no static check can follow (walkthrough 251).
fn inherited_command_words(script: &str) -> (Vec<String>, Vec<String>) {
    let cmds = lex(script);
    let assigned = script_assignments(&cmds);
    let mut found = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for cmd in &cmds {
        let Some(k) = command_word_index(&cmd.words) else {
            continue;
        };
        let word = &cmd.words[k];
        if names_fixed_program(word) {
            continue;
        }
        let text = cmd.words.join(" ");
        // An unquoted use is word split: a value's fixed program name then proves nothing.
        let quoted_use = all_expansions_quoted(word);
        for r in variable_refs(word) {
            let reason = match r {
                VarRef::Special('@') if matches!(unquote(word).as_str(), "$@" | "${@}") => None,
                VarRef::Special(c) => Some(format!("${c} is a command word")),
                VarRef::Indirect(name) => Some(format!("${{!{name}}} is a command word")),
                VarRef::Named(name) => {
                    if !seen.contains(&name) {
                        seen.push(name.clone());
                    }
                    inherited_reason(&name, quoted_use, &assigned, &mut Vec::new())
                }
            };
            if let Some(reason) = reason {
                found.push(format!("line {}: {reason}: {text}", cmd.line));
            }
        }
    }
    (found, seen)
}

// ---------------------------------------------------------------------------
// Lexer hardening tests (follow-up of the review of PR #460; rows SCN6-SCN10; walkthrough 251)
// ---------------------------------------------------------------------------

/// SCN6: a `tailscale` command word spelled through quotes or escapes (`"tail""scale"`,
/// `\tailscale`, `'tail''scale'`, `t\ailscale`, `$'\x74ailscale'`, `$'\164ailscale'`) is
/// still a Tailscale command for SCN2 (in a substitution) and SCN3 (held in a variable);
/// `tailscale version` spelled the same way, and two separate words `tail` `scale`, are not.
#[test]
fn test_scn6_tailscale_word_found_through_quotes_and_escapes() {
    for (snippet, bad) in [
        ("x=\"$(\"tail\"\"scale\" serve)\"\n", true),
        ("x=$(\\tailscale serve)\n", true),
        ("x=$('tail''scale' up)\n", true),
        ("x=\"$(t\\ails'c'ale funnel reset)\"\n", true),
        ("x=\"$(\"/usr/bin/tail\"scale serve)\"\n", true),
        ("x=\"$($'\\x74ailscale' serve)\"\n", true),
        ("x=\"$($'\\164ailscale' up)\"\n", true),
        ("x=\"$(run_bounded 5 tail\\\n''scale serve)\"\n", true),
        ("x=\"$(\"tail\"\"scale\" version)\"\n", false),
        ("x=\"$('tail''scale' 'ver''sion')\"\n", false),
        ("x=\"$(printf '%s' 'tail' 'scale')\"\n", false),
        ("x=\"$(printf '%s\\n' \"tailscale_version\")\"\n", false),
    ] {
        let found = tailscale_in_substitution_by_any_name(snippet);
        assert_eq!(
            !found.is_empty(),
            bad,
            "SCN2 verdict for {snippet:?} must be {}: {found:?}",
            if bad { "rejected" } else { "accepted" }
        );
    }
    for (snippet, bad) in [
        ("TS='tail''scale'\n", true),
        ("TS=t\"ail\"scale\n", true),
        ("TS=\\tailscale\n", true),
        ("TS=$'\\x74ailscale'\n", true),
        ("CMD=(run_bounded 5 \"tail\"'scale' up)\n", true),
        ("\"${TS:-'tail''scale'}\" serve\n", true),
        ("printf -v TS '%s' 'tail''scale'\n", true),
        ("TS=\"tail\"\"scale version\"\n", false),
        ("parts=(tail scale)\n", false),
    ] {
        let found = tailscale_held_in_variable(snippet);
        assert_eq!(
            !found.is_empty(),
            bad,
            "SCN3 verdict for {snippet:?} must be {}: {found:?}",
            if bad { "rejected" } else { "accepted" }
        );
    }
}

/// SCN7: a command word taken from a variable (`"${TS}" serve`, `run_bounded 5 "$TS" up`,
/// `as_root "${X[@]}"`) names a variable `setup.sh` assigns itself, and every variable its
/// assignments expand is assigned by the script too (transitively, never from itself or from
/// a command's output), so an inherited environment variable never chooses the command
/// `setup.sh` runs; a positional parameter is a command word only as the `"$@"` pass-through of
/// a wrapper. Checked on synthetic input first, then on setup.sh, where the check must see
/// `ADMIN_BIN`, `ENROLL_BIN` and `ESCALATE`.
#[test]
fn test_scn7_command_word_variables_are_assigned_by_the_script() {
    for (snippet, bad) in [
        ("\"${TS}\" serve\n", true),
        ("x=\"$(\"${TS_BIN}\" status)\"\n", true),
        ("run_bounded 5 \"${TS}\" up\n", true),
        ("as_root \"$TS\" up\n", true),
        ("if ! $TS up; then :; fi\n", true),
        ("\"${TOOLS[@]}\" up\n", true),
        ("\"$1\" serve\n", true),
        ("TS=\"${TS:-/usr/bin/true}\"\n\"${TS}\" x\n", true),
        ("X=\"${HOME}\"\n\"${X}\" run\n", true),
        ("A=\"${B}\"\nB=\"${C}\"\n\"${A}\" run\n", true),
        ("X=\"$(printf '%s' \"${TOOL}\")\"\n\"${X}\" run\n", true),
        ("X=\"${!REF}\"\n\"${X}\" run\n", true),
        ("read -r X < f\n\"${X}\" run\n", true),
        ("TS_BIN=\"/usr/bin/true\"\n\"${TS_BIN}\" x\n", false),
        ("ESCALATE=(sudo --)\n\"${ESCALATE[@]}\" \"$@\"\n", false),
        ("run() { \"$@\"; }\n", false),
        (
            "PREFIX=\"/usr\"\nADMIN_BIN=\"${PREFIX}/bin/soos-admin\"\n\"${ADMIN_BIN}\" add-user x\n",
            false,
        ),
        ("echo \"${TS}\"\nx=\"${HOME}\"\n", false),
        (
            "BIN_DIR=\"${HOME}/.local/bin\"\n\"${BIN_DIR}/soos-remote\" x\n",
            false,
        ),
        ("[[ -n \"${A}\" || \"${B}\" == x ]] && :\n", false),
        ("(( ${#A[@]} > 0 )) && :\n", false),
        ("run_bounded \"${QUERY_TIMEOUT_S}\" systemctl is-active x\n", false),
    ] {
        let (found, _) = inherited_command_words(snippet);
        assert_eq!(
            !found.is_empty(),
            bad,
            "scanner verdict for {snippet:?} must be {}: {found:?}",
            if bad { "rejected" } else { "accepted" }
        );
    }
    let (found, seen) = inherited_command_words(&read_repo(SETUP));
    assert!(
        found.is_empty(),
        "setup.sh takes a command word from a variable the environment can set: {found:?}"
    );
    for name in ["ADMIN_BIN", "ENROLL_BIN", "ESCALATE"] {
        assert!(
            seen.iter().any(|s| s == name),
            "the check must see the command-word variable {name}: {seen:?}"
        );
    }
}

/// SCN8: the lexer reads arithmetic (`(( x << 2 ))`, `$(( y << 2 ))`, `for (( ... ))`) as
/// one word, never as a here-document, and a `case ... esac` inside `$( ... )` (labels with or
/// without a leading `(`, `;;`, `;&`, `;;&`, nested) without ending the substitution at the
/// first label; the commands of such a body are substitution commands that SCN2 checks.
#[test]
fn test_scn8_lexer_reads_arithmetic_and_case_in_substitution() {
    let arith =
        lex("(( x << 2 )) && y=$(( z << 3 ))\nfor (( i = 0; i << 1; i++ )); do :; done\nok\n");
    assert_eq!(arith.last().map(|c| c.line), Some(3), "{arith:?}");
    assert!(
        arith.iter().any(|c| c.words == ["(( x << 2 ))"]),
        "{arith:?}"
    );
    assert!(
        arith.iter().any(|c| c.words == ["y=$(( z << 3 ))"]),
        "{arith:?}"
    );
    assert!(
        arith
            .iter()
            .any(|c| c.words[0] == "for" && c.words.len() == 2),
        "{arith:?}"
    );
    let inner = lex("x=\"$(( $(a 1) << 2 ))\"\n");
    assert!(
        inner.iter().any(|c| c.words[0] == "a" && c.in_subst),
        "{inner:?}"
    );
    let subshell = lex("x=$((a) | b)\n((c) && d)\n");
    assert!(
        subshell.iter().any(|c| c.words == ["a"] && c.in_subst),
        "{subshell:?}"
    );
    assert!(subshell.iter().any(|c| c.words == ["d"]), "{subshell:?}");

    let case =
        lex("x=\"$(case \"$1\" in a) echo \"a\" ;; (b|c) echo b ;& d) e ;;& *) f ;; esac)\"\nok\n");
    for name in ["echo", "e", "f"] {
        assert!(
            case.iter().any(|c| c.words[0] == name && c.in_subst),
            "{name} is a substitution command: {case:?}"
        );
    }
    assert!(
        case.iter().any(|c| c.words == ["ok"] && !c.in_subst),
        "{case:?}"
    );
    let nested = lex("y=$(case a in a) z=$(case b in b) g ;; esac) ;; esac)\nh\n");
    assert!(
        nested.iter().any(|c| c.words == ["g"] && c.in_subst),
        "{nested:?}"
    );
    assert!(
        nested.iter().any(|c| c.words == ["h"] && !c.in_subst),
        "{nested:?}"
    );
    for snippet in [
        "x=\"$(case \"$1\" in a) tailscale serve ;; esac)\"\n",
        "x=\"$(case \"$1\" in (a) :;; b) \"tail\"scale up ;; esac)\"\n",
        "n=$(( $(tailscale status | wc -l) << 1 ))\n",
    ] {
        assert!(
            !tailscale_in_substitution_by_any_name(snippet).is_empty(),
            "{snippet:?} must be rejected"
        );
    }
    assert!(tailscale_in_substitution_by_any_name(
        "x=\"$(case \"$1\" in a) tailscale version ;; esac)\"\n"
    )
    .is_empty());
}

/// SCN9: the lexer never panics: `try_lex` returns `Ok` for the inputs of SCN8 and `Err`
/// (fail closed, never a silent pass) for unbalanced, truncated or too deeply nested input,
/// and returns without a panic or a stack overflow on a deterministic batch of generated
/// inputs; `lex`, which every scanner uses, turns that `Err` into a test failure naming the
/// problem.
#[test]
fn test_scn9_lexer_never_panics_and_fails_closed() {
    for good in [
        "(( x << 2 ))\n",
        "y=$(( z << 3 ))\n",
        "x=\"$(case \"$1\" in a) echo a ;; esac)\"\n",
        "x=$(case a in (a) b ;; esac)\n",
    ] {
        let result = std::panic::catch_unwind(|| try_lex(good));
        assert!(
            matches!(result, Ok(Ok(_))),
            "{good:?} must lex without a panic"
        );
    }
    let mut bad: Vec<String> = [
        "x=\"$(a\n",
        "x='a\n",
        "cat <<EOF\nbody\n",
        "y=`b\n",
        "f() { (a\n",
        "x=$'a\\",
        "x=\"a\\",
        "x=${a\\",
        "cat <<\n",
        "cat <<'EOF\n",
        "x=$(case a in a) b\n",
        "(( x << 2 )\n",
        "x=$(( y << 2 )\n",
        "x=$((",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    bad.extend(["$(", "${", "\"$(", "$(("].map(|open| open.repeat(100_000)));
    for input in &bad {
        let result = std::panic::catch_unwind(|| try_lex(input));
        let shown: String = input.chars().take(24).collect();
        match result {
            Ok(Err(e)) => assert!(!e.is_empty(), "{shown:?}: empty error"),
            Ok(Ok(cmds)) => panic!("{shown:?} must fail closed, got {cmds:?}"),
            Err(_) => panic!("{shown:?} made the lexer panic"),
        }
    }
    let lexed = std::panic::catch_unwind(|| lex("x=\"$(a\n"));
    let message = lexed
        .err()
        .and_then(|p| p.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    assert!(
        message.contains("unterminated"),
        "lex fails with a clear message: {message:?}"
    );

    let alphabet: Vec<char> = "ab $(){}[]'\"`\\\n<>|&;#=-!*?@:+%^~/\t0".chars().collect();
    let mut seed: u64 = 0x5eed_5c4a_11e5;
    let mut next = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(seed >> 33).unwrap_or(0)
    };
    for _ in 0..4000 {
        let len = next() % 96 + 1;
        let input: String = (0..len)
            .map(|_| alphabet[next() % alphabet.len()])
            .collect();
        let result = std::panic::catch_unwind(|| try_lex(&input));
        assert!(result.is_ok(), "{input:?} made the lexer panic");
    }
}

/// SCN10: traceability: the matrix carries SCN6-SCN10, walkthrough 251 exists, and SCN1-SCN5
/// are kept.
#[test]
fn test_scn10_traceability() {
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for row in 6..=10 {
        assert!(
            matrix.contains(&format!("| SCN{row} |")),
            "matrix row SCN{row}"
        );
    }
    assert!(workspace_root()
        .join("AI/walkthroughs/251_setup_scanner_lexer.md")
        .is_file());
    let contract = read_repo("tests/invariants/src/setup_scanner_contract.rs");
    for test in [
        "fn test_scn1_lexer_joins_continuations_and_follows_contexts()",
        "fn test_scn2_no_tailscale_in_substitution_by_path_or_quoted_name()",
        "fn test_scn3_no_tailscale_command_held_in_a_variable()",
        "fn test_scn4_daemon_unit_bound_survives_continuation_lines()",
        "fn test_scn5_traceability()",
    ] {
        assert!(contract.contains(test), "{test} is kept");
    }
}

/// SCN11: `[[` opens a test only in command-word position (first word, or after `if`,
/// `elif`, `while`, `until`, `!`, `then`, `do`, `else`, `{`, `time`): an argument `[[`
/// (`echo [[`) never merges the following lines into one command, so SCN4 still sees an
/// unbounded daemon unit call after it; a test left open when a substitution ends, or cut by
/// `;`, is an `Err` (fail closed). Follow-up of the review of commit 980b2ca.
#[test]
fn test_scn11_test_mode_only_in_command_position() {
    for snippet in [
        "echo [[\nrun_bounded \"${UNIT_START_TIMEOUT_S}\"\nsystemctl start soos-daemon\necho ]]\n",
        "x=\"$(echo [[\nsystemctl start soos-daemon\n)\"\n",
        "x=`printf '%s' [[\nsystemctl stop \"${UNIT}\"\n`\n",
        "if [[ -n x ||\n  -n y ]]; then systemctl start soos-daemon; fi\n",
    ] {
        let (found, _) = daemon_unit_call_violations(snippet);
        assert!(
            !found.is_empty(),
            "SCN4 must still reject {snippet:?}: {found:?}"
        );
    }
    let ok = lex("if ! [[ a && ( b || c ) ]]; then :; fi\nwhile [[ -n x ]]; do :; done\n");
    assert!(
        ok.iter()
            .any(|c| c.words.first().map(String::as_str) == Some("if")
                && c.words.iter().any(|w| w == "]]")),
        "{ok:?}"
    );
    for bad in [
        "x=\"$([[ -n a\n)\"\n",
        "x=`[[ -n a`\n",
        "[[ -n a ; ]]\n",
        "[[ -n a\n",
    ] {
        let result = std::panic::catch_unwind(|| try_lex(bad));
        assert!(
            matches!(result, Ok(Err(_))),
            "{bad:?} must fail closed without a panic"
        );
    }
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for row in ["| SCN11 |", "| SCN12 |"] {
        assert!(matrix.contains(row), "matrix row {row}");
    }
}

/// SCN12: SCN7's fixed-program exception holds only when every expansion of the command
/// word (and of the variable's value) is double-quoted, since an unquoted expansion is word
/// split (`TS='tailscale serve '` makes `$TS/x run` run `tailscale`); `eval`, `xargs`,
/// `nice`, `setsid` and `stdbuf` are followed like the other wrappers.
#[test]
fn test_scn12_command_word_quoting_and_more_wrappers() {
    for (snippet, bad) in [
        ("$TS/x run\n", true),
        ("${TS}/bin/x run\n", true),
        ("X=\"$TS/bin/tool\"\n$X up\n", true),
        ("X=$TS/bin/tool\n\"${X}\" up\n", true),
        ("eval \"$TS\"\n", true),
        ("xargs -0 \"$TS\" < list\n", true),
        ("nice -n 5 \"${TS}\" up\n", true),
        ("setsid \"$TS\" up\n", true),
        ("stdbuf -oL \"$TS\" up\n", true),
        ("\"$TS/x\" run\n", false),
        ("X=\"${P}/bin/tool\"\n\"${X}\" up\n", false),
        ("A=\"/usr/bin/a\"\nnice -n 5 \"${A}\" x\n", false),
        ("eval \"echo hi\"\n", false),
    ] {
        let (found, _) = inherited_command_words(snippet);
        assert_eq!(
            !found.is_empty(),
            bad,
            "scanner verdict for {snippet:?} must be {}: {found:?}",
            if bad { "rejected" } else { "accepted" }
        );
    }
    let (found, _) = inherited_command_words(&read_repo(SETUP));
    assert!(found.is_empty(), "setup.sh: {found:?}");
}
