"""Tests for the `fleet wsl <device> restart/exec` command sent to a Windows
gateway over `ssh_exec(..., raw=True)`.

Background (see docs/reports/wsl2-local-ssh-outage-2026-09-06.md in
seeed-solutions-hub): `cmd_wsl`'s restart and exec actions used to build
`wsl <flags> -e bash -c '<POSIX-quoted command>'` and send it verbatim to a
Windows gateway. `raw=True` means the string is handed straight to the
gateway's `cmd.exe`, which does not treat single quotes as a grouping
character — bash only ever saw the opening quote and reported
"unexpected EOF while looking for matching ''".

Simply switching to cmd.exe-style double quotes is not enough either:
cmd.exe expands `%VAR%` even *inside* double-quoted segments, so a payload
containing e.g. `%PATH%` would get mangled before bash ever sees it. The fix
sends the inner command base64-encoded (an alphabet cmd.exe never touches)
and decodes it on the WSL side, so nothing about the payload is visible to
cmd.exe's tokenizer at all.

Run with:  uv run --with pytest --with paramiko pytest fleet_backend/test_wsl_quoting.py
"""
import base64
import os
import importlib.util
import pathlib
import re
import sys

_SPEC = importlib.util.spec_from_file_location(
    "fleet_backend_fleet_wsl", pathlib.Path(__file__).with_name("fleet.py")
)
fleet = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = fleet
_SPEC.loader.exec_module(fleet)

build = fleet.wsl_exec_command


CASES = [
    "sudo service ssh start; echo WSL_STARTED",
    "echo it's fine",                       # single quote
    'echo "quoted"',                         # double quote
    "echo $HOME and $(id -u)",               # dollar / substitution
    "echo %PATH% and %USERPROFILE%",         # percent (cmd.exe var expansion)
    "echo hello world with spaces",          # spaces
    "printf 'a|b & c > d < e ^ f'",          # cmd.exe metacharacters
    "echo line1\necho line2\necho line3",    # embedded newlines / multi-statement
    "echo 你好世界 && echo 日本語",             # non-ASCII (CJK)
]


def _cmd_exe_would_mangle(built_command: str) -> bool:
    """A deliberately simplified model of the two cmd.exe hazards this fix
    defends against — not a full cmd.exe parser, just enough to prove the
    old POSIX-single-quote approach breaks and the new one does not:

    1. A single quote (') has no grouping meaning to cmd.exe at all, so if
       one appears anywhere outside our own wrapper text, cmd.exe has
       already split/passed the line in a way bash never intended.
    2. `%NAME%` is expanded by cmd.exe even inside double quotes, so any
       literal `%...%` pair appearing in the command text is unsafe.
    """
    if "'" in built_command:
        return True
    if re.search(r"%[A-Za-z_][A-Za-z0-9_]*%", built_command):
        return True
    return False


def test_helper_exists_and_builds_a_string():
    # Fails with AttributeError before the fix (no wsl_exec_command yet).
    assert isinstance(build("-d Ubuntu", "echo ok"), str)


def test_restart_command_has_no_raw_single_quotes():
    built = build("-d Ubuntu", "sudo service ssh start; echo WSL_STARTED")
    assert not _cmd_exe_would_mangle(built)


def test_payloads_survive_the_simplified_cmd_exe_hazard_check():
    for payload in CASES:
        built = build("-d Ubuntu", payload)
        assert not _cmd_exe_would_mangle(built), f"unsafe for cmd.exe: {built!r}"


def test_payload_round_trips_through_base64():
    for payload in CASES:
        built = build("-d Ubuntu", payload)
        match = re.search(r"printf %s ([A-Za-z0-9+/=]+) \| base64 -d", built)
        assert match, f"no base64 payload found in {built!r}"
        decoded = base64.b64decode(match.group(1)).decode("utf-8")
        assert decoded == payload


def test_empty_distro_flag_still_produces_a_valid_command():
    built = build("", "echo ok")
    assert "wsl" in built and "-e bash -c" in built


def test_restart_and_exec_call_sites_use_the_shared_helper():
    """cmd_wsl's restart and exec branches must both go through
    wsl_exec_command instead of hand-rolling their own quoting."""
    import inspect

    src = inspect.getsource(fleet.cmd_wsl)
    assert "wsl_exec_command(" in src
    assert "shlex.quote(inner)" not in src
    assert "bash -c 'sudo service ssh start" not in src


# --- Regression coverage for the 2026-09 codex review of 91236b3 ----------
#
# The original fix (base64 + `echo payload | base64 -d | bash`) sidestepped
# cmd.exe's quoting hazards but introduced its own bugs:
#   1. No length guard — a long enough `inner` could push the whole cmd.exe
#      command line past its ~8191-char limit.
#   2. Piping the decoded script into a shell's *stdin* means whatever
#      `inner` itself wants to read from stdin is stolen by the decode
#      pipeline, and a `base64 -d` failure could be masked by the final
#      stage's exit status.
#   3. Python defaulted to a non-login `bash`, Rust to `bash -l` — same bug
#      class, different remote environment depending on which backend ran.
#
# These tests exercise the fixed helper (temp-file based, not a stdin pipe)
# against those three failure modes, plus the general robustness cases
# codex asked for (newlines, non-ASCII, decode failure, inner exit code,
# inner stdin).


def _extract_inner_script(built_command: str) -> str:
    """Pull the script text passed to the final `bash -c "..."` (the one
    that runs inside WSL) out of a wsl_exec_command() result."""
    match = re.search(r'-e bash -c "(.*)"$', built_command)
    assert match, f"could not find inner bash -c script in {built_command!r}"
    return match.group(1)


def test_decode_does_not_pipe_into_a_shells_stdin():
    """The decoded payload must be written to a file and run as a script
    argument, never piped as `| bash` / `| bash -l`, because piping makes
    the decode pipeline's stdout *become* the inner shell's stdin — stealing
    it from whatever `inner` wants to read from the real terminal/channel."""
    built = build("-d Ubuntu", "cat")
    script = _extract_inner_script(built)
    assert not re.search(r"\|\s*bash", script), f"still piping into a shell: {script!r}"
    assert "base64 -d >" in script  # decoded to a file, not to a pipe


def test_decode_failure_is_not_masked_by_a_later_stage():
    """`base64 -d` failing must short-circuit the `&&` chain so the overall
    exit status reflects the decode failure, not some later stage's success."""
    built = build("-d Ubuntu", "echo ok")
    script = _extract_inner_script(built)
    assert "&&" in script
    setup_stages, _, rest = script.rpartition("&&")
    assert "base64 -d >" in setup_stages
    assert "bash" in rest


def test_default_decode_shell_is_a_login_shell_on_both_call_sites():
    """Python previously defaulted to a non-login `bash`; Rust to `bash -l`.
    Both backends must now agree — a login shell is the default so that
    PATH/profile behave the same as an interactive WSL session regardless
    of which backend built the command."""
    built = build("-d Ubuntu", "echo ok")
    script = _extract_inner_script(built)
    assert "bash -l $f" in script


def test_wsl_and_ssh_user_flag_is_appended_consistently():
    built = build("-d Ubuntu", "echo ok", user_flag="-u root")
    assert built.startswith("wsl -d Ubuntu -u root -e bash -c ")
    # Omitting it entirely must not leave stray double spaces.
    built_no_user = build("-d Ubuntu", "echo ok")
    assert "  " not in built_no_user.split('"', 1)[0]


def test_overlong_payload_is_rejected_before_it_can_overflow_cmd_exe():
    # cmd.exe's line limit is ~8191 chars; MAX_WSL_B64_LEN caps the payload
    # itself well below that, leaving headroom for the wrapper text.
    too_long = "x" * (fleet.MAX_WSL_B64_LEN * 2)
    try:
        build("-d Ubuntu", too_long)
        assert False, "expected ValueError for an over-length payload"
    except ValueError as e:
        assert "base64" in str(e) or "cmd.exe" in str(e)

    # A payload right at the boundary must still be accepted. base64 expands
    # raw bytes by ~4/3, so size the *raw* input accordingly.
    raw_len = (fleet.MAX_WSL_B64_LEN - 100) * 3 // 4
    ok_payload = "x" * raw_len
    build("-d Ubuntu", ok_payload)  # must not raise


def test_two_call_sites_that_pass_a_user_flag_do_not_regress_to_no_user():
    """Both restart's start_cmd and exec's wsl_cmd must forward user_flag
    through to wsl_exec_command (regression guard for the call sites, not
    just the helper itself)."""
    import inspect

    src = inspect.getsource(fleet.cmd_wsl)
    assert src.count("user_flag=user_flag") >= 2


# --- Local execution of the generated script (real bash, no cmd.exe/WSL) --
#
# These run the *inner* `bash -c "<script>"` argument the helper builds,
# via a local bash subprocess, to prove the temp-file dance actually behaves
# as claimed: inner stdin is untouched, the inner exit code passes through,
# and a corrupted payload fails loudly instead of being swallowed. The
# cmd.exe/WSL-specific quoting itself is covered separately by the real
# wsl2-local gateway test (see docs/reports, not part of this file) since
# no Windows box is available in CI.

import shutil
import subprocess

# NB: this repo's own dev environment prepends a fleet-router shim named
# `bash` onto PATH (see ~/.rpty/bin), so `shutil.which("bash")` would find
# the router, not a real shell — hence the hardcoded absolute path.
_REAL_BASH = "/bin/bash"
_HAVE_BASH = os.path.exists(_REAL_BASH)


def _run_inner_script(built_command: str, stdin_text: str | None = None, timeout: float = 5.0):
    script = _extract_inner_script(built_command)
    kwargs = dict(capture_output=True, text=True, timeout=timeout)
    if stdin_text is None:
        # No input to feed: give the child /dev/null so a `bash -l` login
        # shell (or anything else) reading stdin sees EOF immediately
        # instead of blocking on this test process's own (often
        # non-redirected) stdin.
        kwargs["stdin"] = subprocess.DEVNULL
    else:
        kwargs["input"] = stdin_text
    return subprocess.run([_REAL_BASH, "-c", script], **kwargs)


def test_inner_command_can_still_read_real_stdin():
    if not _HAVE_BASH:
        return
    built = build("", "cat")
    result = _run_inner_script(built, stdin_text="hello-from-real-stdin\n")
    assert result.stdout == "hello-from-real-stdin\n", result


def test_inner_exit_code_3_passes_through():
    if not _HAVE_BASH:
        return
    built = build("", "exit 3")
    result = _run_inner_script(built)
    assert result.returncode == 3, result


def test_decode_failure_produces_nonzero_exit_and_keeps_stderr():
    if not _HAVE_BASH:
        return
    built = build("", "echo should-not-run")
    # Corrupt the base64 payload so `base64 -d` fails.
    script = _extract_inner_script(built)
    corrupted = re.sub(
        r"printf %s [A-Za-z0-9+/=]+",
        "printf %s ***not-valid-base64***",
        script,
    )
    result = subprocess.run(
        [_REAL_BASH, "-c", corrupted],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=5.0,
    )
    assert result.returncode != 0
    assert "should-not-run" not in result.stdout
    assert result.stderr.strip() != ""  # base64's error reached real stderr


def test_write_rust_golden_fixture():
    """Regenerate fleet_backend/wsl_exec_command_golden.json, which Rust's
    wsl_exec_command_matches_python_backend_byte_for_byte test reads to
    assert both backends build byte-for-byte identical remote command
    strings for the same inputs. Run this test (or the whole file) after
    changing wsl_exec_command in either language, then re-run `cargo test`."""
    import json as _json

    fixture_cases = []
    matrix = [
        {"distro_flag": "-d Ubuntu", "user_flag": ""},
        {"distro_flag": "", "user_flag": ""},
        {"distro_flag": "-d Ubuntu", "user_flag": "-u root"},
    ]
    for combo in matrix:
        for payload in CASES:
            expected = build(
                combo["distro_flag"], payload, user_flag=combo["user_flag"]
            )
            fixture_cases.append(
                {
                    "distro_flag": combo["distro_flag"],
                    "inner": payload,
                    "decode_shell": "bash -l",
                    "user_flag": combo["user_flag"],
                    "expected": expected,
                }
            )

    out_path = pathlib.Path(__file__).with_name("wsl_exec_command_golden.json")
    out_path.write_text(_json.dumps(fixture_cases, indent=2, ensure_ascii=False) + "\n")
    assert out_path.exists()


def test_multiline_and_non_ascii_payloads_execute_correctly_locally():
    if not _HAVE_BASH:
        return
    built = build("", "echo line1\necho 你好世界")
    result = _run_inner_script(built)
    assert result.returncode == 0
    assert result.stdout == "line1\n你好世界\n"
