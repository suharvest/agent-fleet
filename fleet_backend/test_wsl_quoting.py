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
        match = re.search(r"echo ([A-Za-z0-9+/=]+) \| base64 -d", built)
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
