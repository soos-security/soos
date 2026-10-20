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

/// The words of the simple command being read.
#[derive(Default)]
struct Acc {
    words: Vec<String>,
    word: String,
    started: bool,
    first: Option<usize>,
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
        }
    }
}

struct Lexer<'a> {
    chars: &'a [char],
    /// 1-based line of every char index.
    line_of: Vec<usize>,
    cmds: Vec<ShCmd>,
    /// Here-documents opened on the current line: (delimiter, expansions active).
    pending: Vec<(String, bool)>,
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
        while i < end {
            if self.chars[i] == '\\' && self.chars.get(i + 1) == Some(&'\n') {
                i += 2;
                continue;
            }
            out.push(self.chars[i]);
            i += 1;
        }
        out
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
    fn single(&self, mut i: usize, opened: usize) -> usize {
        while i < self.chars.len() {
            if self.chars[i] == '\'' {
                return i + 1;
            }
            i += 1;
        }
        panic!(
            "unterminated single quote opened on line {}",
            self.line(opened)
        );
    }

    /// `$'...'` starting after the opening quote (backslash escapes active).
    fn ansi_c(&self, mut i: usize, opened: usize) -> usize {
        while i < self.chars.len() {
            match self.chars[i] {
                '\\' => i += 2,
                '\'' => return i + 1,
                _ => i += 1,
            }
        }
        panic!("unterminated $'...' opened on line {}", self.line(opened));
    }

    /// `"..."` starting after the opening quote; substitutions inside are lexed.
    fn double(&mut self, mut i: usize, opened: usize) -> usize {
        while i < self.chars.len() {
            let next = self.chars.get(i + 1).copied();
            match self.chars[i] {
                '"' => return i + 1,
                '\\' => i += 2,
                '$' if next == Some('(') => i = self.list(i + 2, true, Term::Paren),
                '$' if next == Some('{') => i = self.brace(i + 2, i),
                '`' => i = self.list(i + 1, true, Term::Backtick),
                _ => i += 1,
            }
        }
        panic!(
            "unterminated double quote opened on line {}",
            self.line(opened)
        );
    }

    /// `${...}` starting after the opening brace (quotes and substitutions inside followed,
    /// as SET41's scanner does).
    fn brace(&mut self, mut i: usize, opened: usize) -> usize {
        while i < self.chars.len() {
            let next = self.chars.get(i + 1).copied();
            match self.chars[i] {
                '}' => return i + 1,
                '\\' => i += 2,
                '\'' => i = self.single(i + 1, i),
                '"' => i = self.double(i + 1, i),
                '$' if next == Some('(') => i = self.list(i + 2, true, Term::Paren),
                '$' if next == Some('{') => i = self.brace(i + 2, i),
                '`' => i = self.list(i + 1, true, Term::Backtick),
                _ => i += 1,
            }
        }
        panic!("unterminated ${{ opened on line {}", self.line(opened));
    }

    /// Bodies of the here-documents opened on the line that just ended, starting at `i`.
    fn heredoc_bodies(&mut self, mut i: usize) -> usize {
        for (delim, expand) in std::mem::take(&mut self.pending) {
            let opened = i;
            loop {
                assert!(
                    i < self.chars.len(),
                    "unterminated here-document {delim} (body from line {})",
                    self.line(opened)
                );
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
                        '$' if next == Some('(') => j = self.list(j + 2, true, Term::Paren),
                        '$' if next == Some('{') => j = self.brace(j + 2, j),
                        '`' => j = self.list(j + 1, true, Term::Backtick),
                        _ => j += 1,
                    }
                }
                i = (j.max(end) + 1).min(self.chars.len());
            }
        }
        i
    }

    /// A list of commands from `i` up to `term`; returns the index after the terminator.
    fn list(&mut self, mut i: usize, in_subst: bool, term: Term) -> usize {
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
                assert!(
                    term == Term::Top,
                    "unterminated {term:?} opened on line {}",
                    self.line(opened)
                );
                self.end_cmd(&mut acc, in_subst);
                assert!(
                    paren == 0 && !array && lit_paren == 0,
                    "the script does not end balanced (paren {paren}, array {array}, literal \
                     paren {lit_paren})"
                );
                assert!(
                    self.pending.is_empty(),
                    "here-document never started: {:?}",
                    self.pending
                );
                return i;
            }
            let ch = self.chars[i];
            let next = self.chars.get(i + 1).copied();
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
                '\n' => {
                    self.end_cmd(&mut acc, in_subst);
                    i = self.heredoc_bodies(i + 1);
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
                ';' | '&' | '|' => {
                    self.end_cmd(&mut acc, in_subst);
                    i += 1;
                }
                '<' | '>' if next == Some('(') => {
                    let start = i;
                    i = self.list(i + 2, true, Term::Paren);
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
                    let quoted = matches!(self.chars.get(j), Some('\'' | '"'));
                    let start = if quoted { j + 1 } else { j };
                    let mut end = start;
                    while end < self.chars.len()
                        && (self.chars[end].is_ascii_alphanumeric() || self.chars[end] == '_')
                    {
                        end += 1;
                    }
                    let delim: String = self.chars[start..end].iter().collect();
                    assert!(
                        !delim.is_empty(),
                        "here-document without a delimiter (line {})",
                        self.line(i)
                    );
                    self.pending.push((delim, !quoted));
                    i = if quoted { end + 1 } else { end };
                }
                '<' | '>' => {
                    let text = if next == Some('&') { "&" } else { "" };
                    acc.push(i, &format!("{ch}{text}"));
                    i += 1 + text.len();
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
                ')' if paren > 0 => {
                    self.end_cmd(&mut acc, in_subst);
                    paren -= 1;
                    i += 1;
                }
                ')' if term == Term::Paren => {
                    self.end_cmd(&mut acc, in_subst);
                    return i + 1;
                }
                ')' => {
                    // A `case` pattern label.
                    acc.push(i, ")");
                    self.end_cmd(&mut acc, in_subst);
                    i += 1;
                }
                '`' if term == Term::Backtick => {
                    self.end_cmd(&mut acc, in_subst);
                    return i + 1;
                }
                '`' => {
                    let start = i;
                    i = self.list(i + 1, true, Term::Backtick);
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '$' if next == Some('(') => {
                    let start = i;
                    i = self.list(i + 2, true, Term::Paren);
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '$' if next == Some('{') => {
                    let start = i;
                    i = self.brace(i + 2, i);
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '$' if next == Some('\'') => {
                    let start = i;
                    i = self.ansi_c(i + 2, i);
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '\'' => {
                    let start = i;
                    i = self.single(i + 1, i);
                    let text = self.raw(start, i);
                    acc.push(start, &text);
                }
                '"' => {
                    let start = i;
                    i = self.double(i + 1, i);
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
/// listed on their own, and its raw text also stays in the enclosing word). Panics when the
/// script does not end balanced, so a script the lexer cannot follow fails loudly.
fn lex(script: &str) -> Vec<ShCmd> {
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
    };
    let end = lexer.list(0, false, Term::Top);
    assert_eq!(end, chars.len(), "the lexer stopped before the end");
    lexer.cmds
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

/// Every `tailscale` word of `text` other than `tailscale version`.
fn tailscale_commands(text: &str) -> bool {
    tailscale_tokens(text)
        .into_iter()
        .any(|pos| !followed_by_version(text, pos))
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
