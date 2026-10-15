"""Fault-inject Apple's ticket service without signing or network access."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "staple-macos.sh"


class StapleTests(unittest.TestCase):
    def run_stapler(self, failures=0, invalid=False, validation_failures=0, validate_only=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            xcrun = root / "xcrun"
            xcrun.write_text("""#!/bin/bash
echo "$2" >> "$STATE/calls"
if [[ "$2" == staple ]]; then
    count=0
    [[ ! -f "$STATE/count" ]] || read -r count < "$STATE/count"
    count=$((count + 1))
    echo "$count" > "$STATE/count"
    (( count > FAILURES )) || exit 68
elif [[ "$2" == validate ]]; then
    [[ "$INVALID" != 1 ]] || exit 65
    count=0
    [[ ! -f "$STATE/validations" ]] || read -r count < "$STATE/validations"
    count=$((count + 1))
    echo "$count" > "$STATE/validations"
    (( count > VALIDATION_FAILURES )) || exit 68
fi
""")
            xcrun.chmod(0o755)
            sleep = root / "sleep"
            sleep.write_text("#!/bin/bash\nexit 0\n")
            sleep.chmod(0o755)
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                       STATE=str(root), FAILURES=str(failures), INVALID=str(int(invalid)),
                       VALIDATION_FAILURES=str(validation_failures))
            command = ["bash", str(SCRIPT), str(root / "test image.dmg")]
            if validate_only:
                command.append("--validate-only")
            result = subprocess.run(command,
                                    env=env, capture_output=True, text=True)
            return result.returncode, (root / "calls").read_text().splitlines()

    def test_recovers_after_ticket_delivery_disconnects(self):
        code, calls = self.run_stapler(failures=2)
        self.assertEqual(code, 0)
        self.assertEqual(calls, ["staple", "staple", "staple", "validate"])

    def test_persistent_delivery_failure_does_not_pass(self):
        code, calls = self.run_stapler(failures=99)
        self.assertNotEqual(code, 0)
        self.assertEqual(calls, ["staple"] * 6)

    def test_recovers_when_validation_disconnects_after_stapling(self):
        code, calls = self.run_stapler(validation_failures=1)
        self.assertEqual(code, 0)
        self.assertEqual(calls, ["staple", "validate"] * 2)

    def test_validation_retry_does_not_modify_the_existing_archive(self):
        code, calls = self.run_stapler(validation_failures=1, validate_only=True)
        self.assertEqual(code, 0)
        self.assertEqual(calls, ["validate", "validate"])

    def test_invalid_ticket_does_not_pass(self):
        code, calls = self.run_stapler(invalid=True)
        self.assertNotEqual(code, 0)
        self.assertEqual(calls, ["staple", "validate"] * 6)


if __name__ == "__main__":
    unittest.main()
