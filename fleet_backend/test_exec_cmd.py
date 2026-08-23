"""Tests for `fleet exec` command joining and remote-shell wrapping.

Run with:  uv run --with pytest --with paramiko pytest fleet_backend/test_exec_cmd.py
"""
import importlib.util
import pathlib
import sys

_SPEC = importlib.util.spec_from_file_location(
    "fleet_backend_fleet", pathlib.Path(__file__).with_name("fleet.py")
)
fleet = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = fleet
_SPEC.loader.exec_module(fleet)

join = fleet.join_exec_command
wrap = fleet.wrap_remote_command


def test_single_token_stays_a_shell_snippet():
    assert join(["echo p | tr p q"]) == "echo p | tr p q"


def test_multi_token_preserves_pipe_inside_an_argument():
    assert join(["grep", "-E", "a|b", "f"]) == "grep -E 'a|b' f"


def test_multi_token_preserves_hash_and_semicolon():
    assert join(["echo", "a#b;c"]) == "echo 'a#b;c'"


def test_multi_token_preserves_nested_quotes_and_spaces():
    assert join(["echo", 'he said "hi"', "/tmp/my dir/x"]) == (
        "echo 'he said \"hi\"' '/tmp/my dir/x'"
    )
    assert join(["echo", "it's"]) == "echo 'it'\"'\"'s'"


def test_shell_flag_restores_naive_join():
    assert join(["echo", "a#b;c"], shell=True) == "echo a#b;c"


def test_literal_wins_over_shell():
    assert join(["echo", "a#b;c"], literal=True, shell=True) == "echo 'a#b;c'"


def test_literal_shlex_joins_a_single_token():
    assert join(["echo p | tr p q"], literal=True) == "'echo p | tr p q'"


def test_plain_wrap_quotes_the_whole_command():
    wrapped = wrap("echo 'a#b;c'")
    assert wrapped.startswith("bash -c '")
    assert "export PATH=" in wrapped
    assert "; echo 'a#b;c'" not in wrapped


def test_sudo_wrap_covers_the_entire_command():
    wrapped = wrap("id -u; id -u", sudo=True)
    assert wrapped == (
        "sudo -S -p '' env DEBIAN_FRONTEND=noninteractive "
        'PATH="$HOME/.local/bin:$PATH" bash -c \'id -u; id -u\''
    )
    assert not wrapped.endswith("; id -u")


def test_raw_and_windows_are_left_verbatim():
    assert wrap("dir C:\\", raw=True) == "dir C:\\"
    assert wrap("dir C:\\", windows=True) == "dir C:\\"


def test_build_remote_command_composes_join_and_wrap():
    assert fleet.build_remote_command(["echo", "a#b;c"]) == wrap("echo 'a#b;c'")


def test_trailing_ampersand_detection():
    assert fleet.has_trailing_ampersand("sleep 30 &")
    assert fleet.has_trailing_ampersand("sleep 30 &   ")
    assert not fleet.has_trailing_ampersand("true && echo ok")
    assert not fleet.has_trailing_ampersand("echo done")
