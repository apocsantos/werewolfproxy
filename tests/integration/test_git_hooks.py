"""Exercise installed hooks with disposable repositories and local pushes."""
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class GitHooks(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="werewolf hooks ")
        self.addCleanup(self.tmp.cleanup)
        self.base = pathlib.Path(self.tmp.name)
        self.repo = self.base / "checkout"
        self.repo.mkdir()
        # Git exports repository variables to hooks. Do not let those or user
        # configuration redirect these tests into the real checkout or hooks.
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        self.env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
                        GIT_AUTHOR_NAME="Hook Test", GIT_AUTHOR_EMAIL="test@example.invalid",
                        GIT_COMMITTER_NAME="Hook Test", GIT_COMMITTER_EMAIL="test@example.invalid")
        self.run_ok(["git", "init", "--template=", "-b", "main"], self.repo)
        scripts = self.repo / "scripts"
        scripts.mkdir()
        shutil.copy2(ROOT / "scripts/install-git-hooks.sh", scripts)
        self.write_script(scripts / "ci-local.sh", '''#!/usr/bin/env bash
set -eu
printf '%s\\n' "$PWD" "$#" "$@" >> "$HOOK_CALLS"
exit "${HOOK_RESULT:-0}"
''')
        # These commands must never execute, even when they are available.
        poison = '''#!/usr/bin/env bash
echo "$0 $*" >> "$HOOK_FORBIDDEN"
exit 91
'''
        for name in ("test-targets.sh", "quick-gate.sh", "json-contract-gate.sh", "schema-gate.sh"):
            self.write_script(scripts / name, poison)
        binaries = self.base / "bin"
        binaries.mkdir()
        for name in ("wolf-b", "cargo"):
            self.write_script(binaries / name, poison)
        self.env["PATH"] = str(binaries) + os.pathsep + self.env["PATH"]
        self.calls = self.base / "calls"
        self.forbidden = self.base / "forbidden"
        self.env.update(HOOK_CALLS=str(self.calls), HOOK_FORBIDDEN=str(self.forbidden))
        self.run_ok(["git", "add", "scripts"], self.repo)
        self.run_ok(["git", "commit", "-m", "fixture"], self.repo)

    def tearDown(self):
        self.assertFalse(self.forbidden.exists(), "hook invoked a legacy/live-node command")

    @staticmethod
    def write_script(path, body):
        path.write_text(body)
        path.chmod(0o755)

    def run_command(self, args, cwd, **kwargs):
        return subprocess.run(args, cwd=cwd, env=self.env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              timeout=20, **kwargs)

    def run_ok(self, args, cwd):
        result = self.run_command(args, cwd)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def install(self, repo=None, cwd=None):
        repo = repo or self.repo
        return self.run_ok(["bash", str(repo / "scripts/install-git-hooks.sh")], cwd or repo)

    def remote(self):
        remote = self.base / "remote.git"
        self.run_ok(["git", "init", "--bare", "--template=", str(remote)], self.base)
        self.run_ok(["git", "remote", "add", "origin", str(remote)], self.repo)
        return remote

    def test_push_runs_only_fast_local_gate_and_blocks_failure(self):
        remote = self.remote()
        self.install()
        self.run_ok(["git", "push", "origin", "HEAD:main"], self.repo)
        self.assertEqual(self.calls.read_text().splitlines(), [str(self.repo), "1", "--fast"])
        before = self.run_ok(["git", "rev-parse", "refs/heads/main"], remote).stdout
        self.run_ok(["git", "commit", "--allow-empty", "-m", "second"], self.repo)
        self.env["HOOK_RESULT"] = "23"
        result = self.run_command(["git", "push", "origin", "HEAD:main"], self.repo)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.run_ok(["git", "rev-parse", "refs/heads/main"], remote).stdout, before)
        # Direct invocation preserves the gate's exact failure status too.
        result = self.run_command([str(self.repo / ".git/hooks/pre-push")], self.repo)
        self.assertEqual(result.returncode, 23)
        self.assertEqual(len(self.calls.read_text().splitlines()), 9)

    def test_install_preserves_previous_hook_and_is_idempotent(self):
        hook = self.repo / ".git/hooks/pre-push"
        hook.parent.mkdir()
        previous = "#!/bin/sh\nexit 74\n"
        self.write_script(hook, previous)
        self.install(cwd=self.base)
        backups = list(hook.parent.glob("pre-push.backup.*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_text(), previous)
        before = hook.stat().st_ino
        self.install(cwd=self.repo / "scripts")
        self.assertEqual(hook.stat().st_ino, before)
        self.assertEqual(list(hook.parent.glob("pre-push.backup.*")), backups)
        self.assertEqual(list(hook.parent.glob("pre-push.tmp.*")), [])
        self.assertTrue(os.access(hook, os.X_OK))

    def test_configured_hook_paths(self):
        for location in ("custom hooks", str(self.base / "absolute hooks")):
            with self.subTest(location=location):
                self.run_ok(["git", "config", "core.hooksPath", location], self.repo)
                self.install()
                hook = self.repo / location / "pre-push"
                self.assertTrue(hook.is_file())
                self.run_ok([str(hook)], self.repo)
        self.assertEqual(self.calls.read_text().splitlines(), [str(self.repo), "1", "--fast"] * 2)
        self.assertFalse((self.repo / ".git/hooks/pre-push").exists())

    def test_shared_hook_uses_pushing_worktree(self):
        self.remote()
        worktree = self.base / "linked checkout"
        self.run_ok(["git", "worktree", "add", "-b", "linked", str(worktree)], self.repo)
        self.install(repo=worktree, cwd=worktree / "scripts")
        self.assertTrue((worktree / ".git").is_file())
        self.assertTrue((self.repo / ".git/hooks/pre-push").is_file())
        self.run_ok(["git", "push", "origin", "HEAD:linked"], worktree)
        self.run_ok(["git", "push", "origin", "HEAD:main"], self.repo)
        self.assertEqual(self.calls.read_text().splitlines(),
                         [str(worktree), "1", "--fast", str(self.repo), "1", "--fast"])


if __name__ == "__main__":
    unittest.main()
