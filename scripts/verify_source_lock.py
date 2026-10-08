#!/usr/bin/env python3
"""Compatibility entry point for the shared offline source-lock verifier.

CI installs Node before this step. Keeping one implementation also permits the
same checks on Windows machines whose Python command is only a Store alias.
"""
import subprocess
import sys
from pathlib import Path


if __name__ == "__main__":
    verifier = Path(__file__).with_suffix(".mjs")
    sys.exit(subprocess.call(["node", str(verifier)]))
