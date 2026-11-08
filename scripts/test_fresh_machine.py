"""Tests for the new-computer test's pieces that need no kwnt binary (they run in the `site` job's unittest step).

- the stand-in curl serves the installer URL and refuses (and logs) every other URL;
- the stand-in installer, run the way kwnt runs it (`curl -fsSL URL | bash`, bare PATH, empty HOME), leaves a signed-out
  `claude` that prints an allowed sign-in link;
- every check in release_required_checks.txt is a job in ci.yml, so the release gate can't wait for a name that
  never reports.
"""
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import fresh_machine_test as fm  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent


class StandInDownload(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home, self.bin, _, self.log = fm.make_machine(Path(self.tmp.name) / "m")
        self.env = {"HOME": str(self.home), "PATH": str(self.bin)}

    def tearDown(self):
        self.tmp.cleanup()

    def run_sh(self, cmd):
        return subprocess.run(["sh", "-c", cmd], env=self.env, capture_output=True, text=True, timeout=30)

    def test_other_urls_are_refused_and_logged(self):
        r = self.run_sh("curl -fsSL https://x.ai/cli/install.sh")
        self.assertEqual(r.returncode, 22)
        self.assertIn("refused", r.stderr)
        self.assertEqual(self.log.read_text().split(), ["https://x.ai/cli/install.sh"])

    def test_install_then_signed_out_claude_with_a_link(self):
        r = self.run_sh(f"curl -fsSL {fm.INSTALL_URL} | bash")
        self.assertEqual(r.returncode, 0, r.stderr)
        claude = self.home / ".local" / "bin" / "claude"
        self.assertTrue(claude.is_file())
        status = subprocess.run([str(claude), "auth", "status", "--json"], env=self.env, capture_output=True, text=True)
        self.assertEqual(status.stdout.strip(), '{"loggedIn":false}')
        login = subprocess.Popen([str(claude), "auth", "login", "--claudeai"], env=self.env, stdout=subprocess.PIPE, text=True)
        try:
            text = "".join(login.stdout.readline() for _ in range(2))
        finally:
            login.kill()
        self.assertRegex(text, r"https://(claude\.ai|claude\.com)/")
        self.assertEqual(self.log.read_text().split(), [fm.INSTALL_URL])

    def test_no_ai_tools_on_the_bare_path(self):
        for tool in ("claude", "codex", "grok", "node", "npm", "brew", "apt-get", "docker"):
            self.assertFalse((self.bin / tool).exists(), tool)


class RequiredChecks(unittest.TestCase):
    def test_every_required_check_is_a_ci_job(self):
        names = [l.strip() for l in (ROOT / "scripts" / "release_required_checks.txt").read_text().splitlines()
                 if l.strip() and not l.startswith("#")]
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text()
        jobs = set(re.findall(r"^    name: (\S+)\s*$", ci, re.M))
        self.assertIn("fresh-machine", names)
        for n in names:
            self.assertIn(n, jobs, f"{n} is required for a release but ci.yml has no job named that")


if __name__ == "__main__":
    unittest.main()
