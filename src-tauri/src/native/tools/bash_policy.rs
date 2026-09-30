//! Bash 命令策略表：按命令、子命令和选项判断单条命令是否只读或有破坏性。
//!
//! 输入是 [`super::shell_parse`] 切好的参数。只读判断采用白名单：
//! 未列出的命令、子命令或选项一律视为不确定，由调用方要求确认。
//! 本机会导入用户的 shell 别名与函数，SSH 执行不套沙箱，所以这里的
//! 结论只用于权限提示，不等于 OS 级隔离。

use super::permission::NativeToolRiskKind;

/// 去掉路径和前导反斜杠后的命令名（`/bin/rm`、`\rm` → `rm`）。
pub fn command_name(token: &str) -> &str {
    let stripped = token.trim_start_matches('\\');
    stripped
        .rsplit(['/', '\\'])
        .next()
        .filter(|item| !item.is_empty())
        .unwrap_or(stripped)
}

/// 跳过 `nohup`、`env`、`timeout` 等包装命令，返回真实命令的起始下标，
/// 以及是否经过了包装（包装可能写文件或改变可执行文件查找）。
pub fn unwrap_command(argv: &[String]) -> (usize, bool) {
    let mut index = 0usize;
    while index < argv.len() {
        let skip_options = |mut index: usize| {
            while index < argv.len() && argv[index].starts_with('-') {
                index += 1;
            }
            index
        };
        index = match command_name(&argv[index]) {
            "env" => {
                let mut next = index + 1;
                while next < argv.len() && (argv[next].starts_with('-') || argv[next].contains('='))
                {
                    next += 1;
                }
                // 不带命令的 `env` 只是打印环境变量。
                if next >= argv.len() {
                    return (index, index > 0);
                }
                next
            }
            "nohup" | "time" | "chronic" => index + 1,
            "command" | "stdbuf" => skip_options(index + 1),
            "nice" => match argv.get(index + 1).map(String::as_str) {
                Some("-n") => index + 3,
                Some(option) if option.starts_with('-') => index + 2,
                _ => index + 1,
            },
            "timeout" => {
                let mut next = index + 1;
                while next < argv.len() {
                    let current = argv[next].as_str();
                    if matches!(current, "-k" | "-s" | "--signal" | "--kill-after") {
                        next += 2;
                    } else if current.starts_with('-')
                        || current.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                    {
                        next += 1;
                    } else {
                        break;
                    }
                }
                next
            }
            _ => return (index, index > 0),
        };
    }
    (index.min(argv.len()), index > 0)
}

fn has(argv: &[String], options: &[&str]) -> bool {
    argv.iter().any(|arg| options.contains(&arg.as_str()))
}

/// 选项本身或 `--name=value` 形式。
fn has_prefixed(argv: &[String], options: &[&str]) -> bool {
    argv.iter().any(|arg| {
        options
            .iter()
            .any(|option| arg == option || arg.starts_with(&format!("{option}=")))
    })
}

/// 已知的破坏性操作。返回风险类别与说明。
pub fn dangerous(argv: &[String]) -> Option<(NativeToolRiskKind, &'static str)> {
    use NativeToolRiskKind::*;
    let name = command_name(argv.first()?);
    let risk = match name {
        "eval" | "alias" | "source" | "." | "sudo" | "doas" | "exec" => (Opaque, "不透明命令"),
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" if has(argv, &["-c"]) => {
            (Opaque, "嵌套 shell")
        }
        "python" | "python2" | "python3" | "perl" | "ruby" | "node" | "nodejs" | "php" | "lua"
        | "osascript"
            if has(argv, &["-c", "-e", "-r", "--eval", "-command", "-Command"]) =>
        {
            (Opaque, "解释器内联代码")
        }
        "find" if has(argv, &["-exec", "-execdir", "-ok", "-okdir", "-delete"]) => {
            (Opaque, "find 执行动作")
        }
        "xargs" => (Opaque, "xargs 包装命令"),
        "chmod" if has(argv, &["777", "0777"]) => (Opaque, "chmod 777"),
        "dd" | "mkfs" | "mkfs.ext4" | "mkfs.xfs" | "mkfs.vfat" | "mkfs.ntfs" => {
            (Opaque, "磁盘危险操作")
        }
        "rm" | "rmdir" | "unlink" | "shred" => (Delete, "删除"),
        "cp" | "mv" | "install" | "tee" | "truncate" | "ln" => (Overwrite, "可能覆盖文件"),
        "git" => return dangerous_git(argv),
        "gh" => return gh_remote_write(argv).then_some((Push, "GitHub 远端写操作")),
        _ => return None,
    };
    Some(risk)
}

fn dangerous_git(argv: &[String]) -> Option<(NativeToolRiskKind, &'static str)> {
    use NativeToolRiskKind::*;
    let (sub, args, _) = git_subcommand(argv)?;
    let risk = match sub {
        "rm" => (Delete, "git rm"),
        "push"
            if has(
                args,
                &[
                    "--force",
                    "-f",
                    "--force-with-lease",
                    "--mirror",
                    "--delete",
                    "-d",
                ],
            ) =>
        {
            (ForceGit, "强制推送")
        }
        "push" => (Push, "推送"),
        "reset" if has(args, &["--hard"]) => (ForceGit, "git reset --hard"),
        "clean"
            if args
                .iter()
                .any(|arg| arg.starts_with('-') && arg.contains('f')) =>
        {
            (ForceGit, "git clean")
        }
        "branch" if has(args, &["-D"]) => (ForceGit, "git branch -D"),
        "checkout" if has(args, &["--", "-f", "--force"]) && args.len() > 1 => {
            (ForceGit, "丢弃改动")
        }
        "restore" => (ForceGit, "git restore"),
        _ => return None,
    };
    Some(risk)
}

/// `git` 的子命令、其后的参数，以及全局选项是否都在已知安全范围内。
/// 全局 `-c`（可注入会执行程序的配置）和未知全局选项会让第三项为假。
fn git_subcommand(argv: &[String]) -> Option<(&str, &[String], bool)> {
    let mut index = 1usize;
    let mut clean = true;
    while index < argv.len() {
        let token = argv[index].as_str();
        match token {
            "-C" | "--git-dir" | "--work-tree" | "--namespace" => index += 2,
            "-c" => {
                clean = false;
                index += 2;
            }
            "--no-pager" | "-P" | "--no-optional-locks" | "--literal-pathspecs" => index += 1,
            _ if token.starts_with("--git-dir=")
                || token.starts_with("--work-tree=")
                || token.starts_with("--namespace=") =>
            {
                index += 1
            }
            _ if token.starts_with('-') => {
                clean = false;
                index += 1;
            }
            _ => return Some((token, &argv[index + 1..], clean)),
        }
    }
    None
}

/// 只读白名单：返回 `Err(原因)` 表示不能确认只读。命令名必须是裸名，
/// 带路径（`/tmp/ls`、`./ls`）的可执行文件不在白名单内。
pub fn read_only(argv: &[String], globbed: bool) -> Result<(), &'static str> {
    let Some(first) = argv.first() else {
        return Ok(());
    };
    if first.contains('/') || first.starts_with('\\') {
        return Err("命令带路径");
    }
    let args = &argv[1..];
    // 这些命令有会执行程序或写文件的选项，未加引号的通配符可能被文件名展开成选项。
    if globbed
        && matches!(
            first.as_str(),
            "rg" | "find" | "sort" | "sed" | "git" | "gh"
        )
    {
        return Err("参数含未加引号的通配符");
    }
    match first.as_str() {
        "echo" | "printf" | "pwd" | "true" | "false" | "cd" | "whoami" | "id" | "uname"
        | "basename" | "dirname" | "readlink" | "realpath" | "ls" | "cat" | "head" | "wc"
        | "stat" | "du" | "df" | "cut" | "tr" | "diff" | "cmp" | "jq" | "which" | "type"
        | "grep" | "egrep" | "fgrep" | "env" => Ok(()),
        "tail" if has_prefixed(args, &["-f", "-F", "--follow", "--retry"]) => {
            Err("持续跟踪不会自行结束")
        }
        "tail" => Ok(()),
        "file" if has_prefixed(args, &["-C", "--compile"]) => Err("会写入 magic 文件"),
        "file" => Ok(()),
        "tree" if has(args, &["-o"]) => Err("会写入输出文件"),
        "tree" => Ok(()),
        "uniq" if args.iter().filter(|arg| !arg.starts_with('-')).count() > 1 => {
            Err("第二个参数是输出文件")
        }
        "uniq" => Ok(()),
        "sort" => {
            let writes = args.iter().any(|arg| {
                arg.starts_with("--output")
                    || arg.starts_with("--compress-program")
                    || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('o'))
            });
            if writes {
                Err("会写入输出文件或执行压缩程序")
            } else {
                Ok(())
            }
        }
        "rg" if has_prefixed(args, &["--pre", "--pre-glob"]) => Err("--pre 会执行外部程序"),
        "rg" => Ok(()),
        "find" if has(args, &["-fprint", "-fprint0", "-fprintf", "-fls"]) => Err("会写入输出文件"),
        "find" => Ok(()),
        "sed" => sed_read_only(args),
        "git" => git_read_only(argv),
        "gh" => gh_read_only(args),
        _ => Err("未在只读白名单中"),
    }
}

/// 只接受 `sed -n '<行号>p'` 这类按行打印的脚本；其余脚本可能含 `w`、`e`。
fn sed_read_only(args: &[String]) -> Result<(), &'static str> {
    let mut quiet = false;
    let mut scripts = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-n" | "--quiet" | "--silent" => quiet = true,
            "-E" | "-r" | "--regexp-extended" => {}
            "-e" | "--expression" => {
                index += 1;
                scripts.push(args.get(index).ok_or("sed 缺少脚本")?.as_str());
            }
            option if option.starts_with('-') => return Err("sed 选项不在只读白名单中"),
            // 没有 `-e` 时第一个位置参数是脚本，其余是输入文件。
            value if scripts.is_empty() => scripts.push(value),
            _ => {}
        }
        index += 1;
    }
    let print_lines = |script: &str| {
        let body = script.strip_suffix('p').unwrap_or("");
        !body.is_empty()
            && body.split(',').count() <= 2
            && body.split(',').all(|part| {
                part == "$" || (!part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
            })
    };
    if quiet && !scripts.is_empty() && scripts.iter().all(|script| print_lines(script)) {
        Ok(())
    } else {
        Err("sed 脚本无法确认只读")
    }
}

fn git_read_only(argv: &[String]) -> Result<(), &'static str> {
    let (sub, args, clean) = git_subcommand(argv).ok_or("缺少 git 子命令")?;
    if !clean {
        return Err("git 全局选项可能执行外部程序");
    }
    if has_prefixed(
        args,
        &[
            "--output",
            "--ext-diff",
            "--textconv",
            "--exec-path",
            "--no-index",
            "--upload-pack",
            "--receive-pack",
            "--exec",
        ],
    ) {
        return Err("git 选项会写文件或执行外部程序");
    }
    let only = |allowed: &[&str]| args.iter().all(|arg| allowed.contains(&arg.as_str()));
    let ok = match sub {
        "status" | "diff" | "log" | "show" | "ls-files" | "rev-parse" | "blame" | "annotate"
        | "describe" | "shortlog" | "ls-remote" | "merge-base" | "cat-file" | "ls-tree"
        | "rev-list" | "show-ref" | "whatchanged" => true,
        "grep" => !args
            .iter()
            .any(|arg| arg.starts_with("-O") || arg.starts_with("--open-files-in-pager")),
        "branch" => only(&[
            "--list",
            "-l",
            "-a",
            "--all",
            "-r",
            "--remotes",
            "-v",
            "-vv",
            "--verbose",
            "--show-current",
            "--no-color",
        ]),
        "remote" => only(&["-v", "--verbose"]),
        "tag" => {
            args.is_empty()
                || (has(args, &["-l", "--list"])
                    && !has(
                        args,
                        &["-d", "--delete", "-a", "-s", "-f", "-m", "-F", "-u"],
                    ))
        }
        "config" => {
            has(
                args,
                &["--get", "--get-all", "--get-regexp", "--list", "-l"],
            ) && !has_prefixed(
                args,
                &[
                    "--unset",
                    "--unset-all",
                    "--add",
                    "--replace-all",
                    "--edit",
                    "-e",
                    "--rename-section",
                    "--remove-section",
                ],
            )
        }
        "reflog" => matches!(args.first().map(String::as_str), None | Some("show")),
        "stash" => matches!(args.first().map(String::as_str), Some("list" | "show")),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err("git 子命令或参数不在只读白名单中")
    }
}

const GH_READ: [(&str, &[&str]); 6] = [
    ("pr", &["view", "list", "diff", "checks", "status"]),
    ("issue", &["view", "list", "status"]),
    ("run", &["view", "list"]),
    ("repo", &["view"]),
    ("release", &["view", "list"]),
    ("auth", &["status"]),
];

const GH_WRITE_ACTIONS: [&str; 24] = [
    "create", "merge", "close", "delete", "edit", "comment", "review", "reopen", "ready", "lock",
    "unlock", "transfer", "archive", "rename", "fork", "sync", "upload", "set", "add", "remove",
    "cancel", "rerun", "pin", "unpin",
];

fn gh_read_only(args: &[String]) -> Result<(), &'static str> {
    if has(args, &["--web", "-w"]) {
        return Err("会打开浏览器");
    }
    let group = args.first().map(String::as_str).unwrap_or("");
    if group == "api" {
        return if gh_api_is_get(&args[1..]) {
            Ok(())
        } else {
            Err("gh api 不是 GET 请求")
        };
    }
    let action = args.get(1).map(String::as_str).unwrap_or("");
    if GH_READ
        .iter()
        .any(|(name, actions)| *name == group && actions.contains(&action))
    {
        Ok(())
    } else {
        Err("gh 子命令不在只读白名单中")
    }
}

/// `gh api` 未指定方法或指定 GET，且没有会把请求变成 POST 的字段参数。
fn gh_api_is_get(args: &[String]) -> bool {
    let mut index = 0usize;
    while index < args.len() {
        let arg = args[index].as_str();
        let method = match arg {
            "-X" | "--method" => {
                index += 1;
                args.get(index).map(String::as_str)
            }
            _ if arg.starts_with("--method=") => arg.strip_prefix("--method="),
            _ if arg.starts_with("-X") => arg.strip_prefix("-X"),
            _ => None,
        };
        if method.is_some_and(|method| !method.eq_ignore_ascii_case("GET")) {
            return false;
        }
        if matches!(arg, "-f" | "-F" | "--field" | "--raw-field" | "--input")
            || arg.starts_with("--field=")
            || arg.starts_with("--raw-field=")
            || arg.starts_with("--input=")
        {
            return false;
        }
        index += 1;
    }
    true
}

/// 会修改 GitHub 远端状态的 gh 调用。
fn gh_remote_write(argv: &[String]) -> bool {
    let args = &argv[1..];
    match args.first().map(String::as_str) {
        Some("api") => !gh_api_is_get(&args[1..]),
        Some(_) => args
            .get(1)
            .is_some_and(|action| GH_WRITE_ACTIONS.contains(&action.as_str())),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(command: &str) -> Vec<String> {
        command.split_whitespace().map(ToOwned::to_owned).collect()
    }

    #[test]
    fn wrappers_are_skipped_and_reported() {
        for (command, start, wrapped) in [
            ("ls -la", 0, false),
            ("env", 0, false),
            ("env -i", 0, false),
            ("env A=1 cat f", 2, true),
            ("nohup cat f", 1, true),
            ("nice -n 5 cat f", 3, true),
            ("timeout -k 1 5 cat f", 4, true),
            ("command -p cat f", 2, true),
            ("time nohup rm x", 2, true),
        ] {
            assert_eq!(
                unwrap_command(&words(command)),
                (start, wrapped),
                "{command}"
            );
        }
        assert_eq!(command_name("/bin/rm"), "rm");
        assert_eq!(command_name("\\rm"), "rm");
    }

    #[test]
    fn path_qualified_commands_are_not_whitelisted() {
        assert!(read_only(&words("ls -la"), false).is_ok());
        assert!(read_only(&words("/tmp/ls"), false).is_err());
        assert!(read_only(&words("./ls"), false).is_err());
        assert!(read_only(&words("make"), false).is_err());
        // 危险判断对带路径的命令仍按命令名生效。
        assert!(dangerous(&words("/bin/rm -rf x")).is_some());
    }
}
