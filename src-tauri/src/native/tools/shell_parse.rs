//! Bash 命令的保守词法分析，供权限分类使用。
//!
//! 只识别分类需要的结构：引号、转义、控制符、重定向和环境变量前缀。
//! 展开、替换、子 shell、分组、heredoc、注释等无法静态确定的结构不做解析，
//! 只记入 `opaque`，由调用方按「不确定」处理。这不是完整的 shell 实现，
//! 也不能代替 OS 沙箱。

/// 重定向的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectOp {
    /// `>`、`>|`、`<>`（读写打开也按写处理）。
    Write,
    /// `>>`。
    Append,
    /// `<`。
    Read,
    /// `>&N` / `<&N` / `N>&-`：复制或关闭文件描述符。
    Dup,
    /// `<<<`：here-string。
    HereString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub op: RedirectOp,
    pub target: String,
}

impl Redirect {
    /// 不会写入工作区文件的重定向。
    pub fn is_harmless(&self) -> bool {
        match self.op {
            RedirectOp::Read | RedirectOp::Dup | RedirectOp::HereString => true,
            RedirectOp::Write | RedirectOp::Append => self.target == "/dev/null",
        }
    }
}

/// 一条简单命令：由 `;`、`&&`、`||`、`|`、`&`、换行分隔。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Segment {
    /// 前置的 `NAME=value`。
    pub assigns: Vec<String>,
    /// 去掉引号和转义后的参数；`argv[0]` 是命令名。
    pub argv: Vec<String>,
    pub redirects: Vec<Redirect>,
    /// 参数里有未加引号的通配符（`*`、`?`、`[`），实际参数取决于文件名。
    pub globbed: bool,
    /// 由管道接收上一段的输出。
    pub piped: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedScript {
    pub segments: Vec<Segment>,
    /// 无法静态确定的结构，非空时整条命令不能视为只读。
    pub opaque: Vec<&'static str>,
}

impl ParsedScript {
    fn mark(&mut self, reason: &'static str) {
        if !self.opaque.contains(&reason) {
            self.opaque.push(reason);
        }
    }
}

#[derive(Default)]
struct Word {
    text: String,
    started: bool,
    /// 第一个带引号或转义的字符在 `text` 中的位置。
    quote_start: Option<usize>,
    globbed: bool,
}

impl Word {
    fn push(&mut self, ch: char) {
        self.text.push(ch);
        self.started = true;
    }

    fn push_quoted(&mut self, ch: char) {
        self.mark_quoted();
        self.push(ch);
    }

    fn mark_quoted(&mut self) {
        self.quote_start.get_or_insert(self.text.len());
        self.started = true;
    }

    fn is_plain_number(&self) -> bool {
        self.started
            && self.quote_start.is_none()
            && !self.text.is_empty()
            && self.text.chars().all(|ch| ch.is_ascii_digit())
    }

    /// `NAME=value`，且 `=` 之前没有引号。
    fn is_assignment(&self) -> bool {
        let Some(eq) = self.text.find('=') else {
            return false;
        };
        let name = &self.text[..eq];
        self.quote_start.is_none_or(|start| start > eq)
            && name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    }
}

/// `>&` 的目标是数字或 `-` 时是复制描述符，否则是同时重定向 stdout/stderr 到文件。
#[derive(Clone, Copy)]
enum Pending {
    Op(RedirectOp),
    DupOrWrite,
}

#[derive(Default)]
struct Builder {
    script: ParsedScript,
    segment: Segment,
    word: Word,
    pending: Option<Pending>,
}

impl Builder {
    fn end_word(&mut self) {
        if !self.word.started {
            return;
        }
        let word = std::mem::take(&mut self.word);
        if let Some(pending) = self.pending.take() {
            if word.globbed {
                self.script.mark("重定向目标含通配符");
            }
            let op = match pending {
                Pending::Op(op) => op,
                Pending::DupOrWrite
                    if word.text == "-" || word.text.chars().all(|ch| ch.is_ascii_digit()) =>
                {
                    RedirectOp::Dup
                }
                Pending::DupOrWrite => RedirectOp::Write,
            };
            self.segment.redirects.push(Redirect {
                op,
                target: word.text,
            });
            return;
        }
        if self.segment.argv.is_empty() && word.is_assignment() {
            self.segment.assigns.push(word.text);
            return;
        }
        self.segment.globbed |= word.globbed;
        self.segment.argv.push(word.text);
    }

    /// 结束当前命令段；`piped` 表示下一段从管道读取。
    fn end_segment(&mut self, piped: bool) {
        self.end_word();
        if self.pending.take().is_some() {
            self.script.mark("重定向缺少目标");
        }
        let segment = std::mem::take(&mut self.segment);
        let empty =
            segment.argv.is_empty() && segment.assigns.is_empty() && segment.redirects.is_empty();
        if empty {
            if piped || segment.piped {
                self.script.mark("管道两侧缺少命令");
            }
        } else {
            self.script.segments.push(segment);
        }
        self.segment.piped = piped;
    }

    fn start_redirect(&mut self, pending: Pending) {
        // `2>`、`1>>` 这类紧贴的数字是文件描述符，不是参数。
        if self.word.is_plain_number() {
            self.word = Word::default();
        } else {
            self.end_word();
        }
        if self.pending.is_some() {
            self.script.mark("重定向缺少目标");
        }
        self.pending = Some(pending);
    }
}

/// 解析整条命令。任何没有把握的结构都记入 `opaque`，不会抛错。
pub fn parse(command: &str) -> ParsedScript {
    let chars: Vec<char> = command.chars().collect();
    let next = |index: usize| chars.get(index + 1).copied();
    let mut b = Builder::default();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            ' ' | '\t' | '\r' => b.end_word(),
            '\n' | ';' => b.end_segment(false),
            '&' if next(i) == Some('&') => {
                i += 1;
                b.end_segment(false);
            }
            '&' if next(i) == Some('>') => {
                i += 1;
                let op = if next(i) == Some('>') {
                    i += 1;
                    RedirectOp::Append
                } else {
                    RedirectOp::Write
                };
                b.start_redirect(Pending::Op(op));
            }
            // 后台执行与顺序执行一样分段。
            '&' => b.end_segment(false),
            '|' if next(i) == Some('|') => {
                i += 1;
                b.end_segment(false);
            }
            '|' => {
                if next(i) == Some('&') {
                    i += 1;
                }
                b.end_segment(true);
            }
            '>' => {
                let pending = match next(i) {
                    Some('>') => {
                        i += 1;
                        Pending::Op(RedirectOp::Append)
                    }
                    Some('|') => {
                        i += 1;
                        Pending::Op(RedirectOp::Write)
                    }
                    Some('&') => {
                        i += 1;
                        Pending::DupOrWrite
                    }
                    _ => Pending::Op(RedirectOp::Write),
                };
                b.start_redirect(pending);
            }
            '<' => {
                let pending = match next(i) {
                    Some('<') if chars.get(i + 2) == Some(&'<') => {
                        i += 2;
                        Pending::Op(RedirectOp::HereString)
                    }
                    Some('<') => {
                        // heredoc 的正文在后续行，不能再按命令解析。
                        b.script.mark("heredoc");
                        i += 1;
                        Pending::Op(RedirectOp::HereString)
                    }
                    Some('&') => {
                        i += 1;
                        Pending::Op(RedirectOp::Dup)
                    }
                    Some('>') => {
                        i += 1;
                        Pending::Op(RedirectOp::Write)
                    }
                    _ => Pending::Op(RedirectOp::Read),
                };
                b.start_redirect(pending);
            }
            '\'' => {
                b.word.mark_quoted();
                match chars[i + 1..].iter().position(|item| *item == '\'') {
                    Some(end) => {
                        for item in &chars[i + 1..i + 1 + end] {
                            b.word.push_quoted(*item);
                        }
                        i += end + 1;
                    }
                    None => {
                        b.script.mark("未闭合的引号");
                        break;
                    }
                }
            }
            '"' => {
                b.word.mark_quoted();
                let mut closed = false;
                i += 1;
                while i < chars.len() {
                    match chars[i] {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' if matches!(next(i), Some('"' | '\\' | '$' | '`')) => {
                            i += 1;
                            b.word.push_quoted(chars[i]);
                        }
                        '\\' if next(i) == Some('\n') => i += 1,
                        '$' | '`' => {
                            b.script.mark("参数展开或命令替换");
                            b.word.push_quoted(chars[i]);
                        }
                        other => b.word.push_quoted(other),
                    }
                    i += 1;
                }
                if !closed {
                    b.script.mark("未闭合的引号");
                    break;
                }
            }
            '\\' => match next(i) {
                // 行尾反斜杠是续行。
                Some('\n') => i += 1,
                Some(item) => {
                    i += 1;
                    b.word.push_quoted(item);
                }
                None => b.word.push('\\'),
            },
            '$' | '`' => {
                b.script.mark("参数展开或命令替换");
                b.word.push(ch);
            }
            '(' | ')' => {
                b.script.mark("子 shell 或进程替换");
                b.word.push(ch);
            }
            '{' | '}' => {
                b.script.mark("花括号分组或展开");
                b.word.push(ch);
            }
            '#' if !b.word.started => {
                b.script.mark("注释");
                while i + 1 < chars.len() && chars[i + 1] != '\n' {
                    i += 1;
                }
            }
            '*' | '?' | '[' => {
                b.word.globbed = true;
                b.word.push(ch);
            }
            other => b.word.push(other),
        }
        i += 1;
    }
    b.end_segment(false);
    b.script
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(command: &str) -> Vec<Vec<String>> {
        parse(command)
            .segments
            .into_iter()
            .map(|segment| segment.argv)
            .collect()
    }

    #[test]
    fn quotes_escapes_and_word_concatenation() {
        assert_eq!(
            argv(r#"grep "a b" file"#),
            vec![vec!["grep", "a b", "file"]]
        );
        assert_eq!(
            argv(r#"echo a"b c"d 'e f'"#),
            vec![vec!["echo", "ab cd", "e f"]]
        );
        assert_eq!(argv(r"echo a\ b"), vec![vec!["echo", "a b"]]);
        assert_eq!(argv("echo '(x)' \"{y}\""), vec![vec!["echo", "(x)", "{y}"]]);
        assert!(parse("echo '(x)' \"{y}\" ';'").opaque.is_empty());
        assert_eq!(argv("echo ''"), vec![vec!["echo", ""]]);
        assert_eq!(argv("ls \\\n  -la"), vec![vec!["ls", "-la"]]);
    }

    #[test]
    fn control_operators_split_segments_and_mark_pipes() {
        let script = parse("a | b && c || d; e & f\ng |& h");
        let names: Vec<&str> = script
            .segments
            .iter()
            .map(|segment| segment.argv[0].as_str())
            .collect();
        assert_eq!(names, ["a", "b", "c", "d", "e", "f", "g", "h"]);
        let piped: Vec<bool> = script
            .segments
            .iter()
            .map(|segment| segment.piped)
            .collect();
        assert_eq!(
            piped,
            [false, true, false, false, false, false, false, true]
        );
        assert!(script.opaque.is_empty());
        assert_eq!(argv("echo 'a;b|c&&d'"), vec![vec!["echo", "a;b|c&&d"]]);
        assert!(!parse("| cat").opaque.is_empty());
    }

    #[test]
    fn redirects_are_separated_from_arguments() {
        let script = parse("cmd arg 2>/dev/null >out.txt 2>&1 <in >>log &>all >&2 1>&- <<<word");
        let segment = &script.segments[0];
        assert_eq!(segment.argv, ["cmd", "arg"]);
        let ops: Vec<(RedirectOp, &str)> = segment
            .redirects
            .iter()
            .map(|item| (item.op, item.target.as_str()))
            .collect();
        assert_eq!(
            ops,
            [
                (RedirectOp::Write, "/dev/null"),
                (RedirectOp::Write, "out.txt"),
                (RedirectOp::Dup, "1"),
                (RedirectOp::Read, "in"),
                (RedirectOp::Append, "log"),
                (RedirectOp::Write, "all"),
                (RedirectOp::Dup, "2"),
                (RedirectOp::Dup, "-"),
                (RedirectOp::HereString, "word"),
            ]
        );
        assert!(segment.redirects[0].is_harmless());
        assert!(!segment.redirects[1].is_harmless());
        assert!(segment.redirects[2].is_harmless());
        // `>&file` 同时写文件；引号里的 `>` 不是重定向；`a>b` 紧贴也识别。
        assert_eq!(
            parse("cmd >&file").segments[0].redirects[0].op,
            RedirectOp::Write
        );
        assert!(parse("echo 'a > b'").segments[0].redirects.is_empty());
        assert_eq!(parse("echo a>b").segments[0].argv, ["echo", "a"]);
        assert!(!parse("cat >").opaque.is_empty());
    }

    #[test]
    fn assignments_and_globs_are_recorded() {
        let segment = &parse("A=1 B=\"x y\" ls *.rs").segments[0];
        assert_eq!(segment.assigns, ["A=1", "B=x y"]);
        assert_eq!(segment.argv, ["ls", "*.rs"]);
        assert!(segment.globbed);
        assert!(!parse("ls '*.rs'").segments[0].globbed);
        // 命令名之后的 `x=y` 是普通参数。
        assert_eq!(parse("echo x=y").segments[0].argv, ["echo", "x=y"]);
        assert!(parse("'A'=1 ls").segments[0].assigns.is_empty());
    }

    #[test]
    fn unresolvable_structures_are_opaque() {
        for command in [
            "echo $(touch f)",
            "echo `id`",
            "echo $HOME",
            "echo \"$HOME\"",
            "echo ${HOME}",
            "(cd src && ls)",
            "{ ls; }",
            "diff <(ls a) <(ls b)",
            "cat <<EOF\nrm -rf /\nEOF",
            "echo 'unterminated",
            "echo \"unterminated",
            "echo #'\ntouch file\n#'",
            "ls > *.txt",
        ] {
            assert!(!parse(command).opaque.is_empty(), "{command}");
        }
        // 注释之后的下一行仍按命令解析。
        let script = parse("echo # note\ntouch file");
        assert_eq!(script.segments.last().unwrap().argv, ["touch", "file"]);
        for command in [
            "ls -la",
            "git log --oneline -n 5",
            "cat a | wc -l",
            "echo a#b",
        ] {
            assert!(parse(command).opaque.is_empty(), "{command}");
        }
    }
}
