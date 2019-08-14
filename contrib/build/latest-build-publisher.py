#!/usr/bin/env python3
# Copyright (c) 2019 The Bitcoin Unlimited developers
# Distributed under the MIT software license, see the accompanying
# file COPYING or http://www.opensource.org/licenses/mit-license.php.

'''
This script polls git for new commits, build electrscash and publishes it.
'''
from datetime import datetime
from utilbuild import cargo_run
import argparse
import http.server
import logging
import os
import sys
import tarfile
import tempfile
import time

v = sys.version_info
if v[0] < 3 or (v[0] == 3 and v[1] < 7):
    print("python >= 3.7 required")
    sys.exit(1)

LOCAL_CLONE_PATH = tempfile.mkdtemp()

parser = argparse.ArgumentParser()
parser.add_argument('--publish-dir',
    help='The directory to be published',
    default=tempfile.mkdtemp())
parser.add_argument('--repo-url', help="URL to the Git repository",
    default='https://github.com/BitcoinUnlimited/ElectrsCash.git')
parser.add_argument('--branch', help="Which branch to follow",
    default="master")
parser.add_argument('--verbose', help='Sets log level to DEBUG',
        action = "store_true")
args = parser.parse_args()

level = logging.DEBUG if args.verbose else logging.INFO
logging.basicConfig(format = '%(levelname)s: %(message)s',
    level=level,
    stream=sys.stdout)

try:
    import git
except Exception as e:
    logging.error("Failed to 'import git'")
    logging.error("Tip: On Debian/Ubuntu you need to install python3-git")
    sys.exit(1)

# Start webserver
class HTTPHandler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *init_args, **init_kwargs):
        super().__init__(*init_args, directory=str(args.publish_dir), **init_kwargs)

def start_web():
    from http.server import HTTPServer
    httpd = http.server.HTTPServer(('', 8000), HTTPHandler)
    return httpd.serve_forever()
import threading
server = threading.Thread(target=start_web)
server.start()
logging.info("Serving %s on port 8000" % args.publish_dir)

# Produce builds

logging.info("Cloning %s %s to %s" % (args.repo_url, args.branch, LOCAL_CLONE_PATH))
repo = git.Repo.clone_from(args.repo_url, LOCAL_CLONE_PATH, branch=args.branch)
last_build = None

wait = 10 * 60

while True:
    repo.remotes.origin.pull()
    if last_build == repo.head.object.hexsha:
        logging.debug("Up-to-date, sleeping %d seconds" % wait)
        import time
        time.sleep(wait)
        continue

    logging.info("Update! Our head %s, remote head %s",
        last_build, repo.head.object.hexsha)

    try:
        cargo_run(["build", "--release", "--target=x86_64-unknown-linux-gnu"],
            logging = logging,
            cwd = LOCAL_CLONE_PATH)
    except Exception as e:
        logging.error("Build failed: %s. Retry in %d seconds" % (e, wait))
        time.sleep(wait)
        continue

    last_build = repo.head.object.hexsha

    archive_name = "electrscash-%s-%s.tar.gz" % (
        datetime.now().strftime("%Y-%m-%d"),
        repo.head.object.hexsha[:8])

    archive_path = os.path.join(args.publish_dir, archive_name)

    built_bin_path = os.path.join(LOCAL_CLONE_PATH,
        "target", "x86_64-unknown-linux-gnu", "release", "electrs")

    logging.info("Compressing %s to %s" % (built_bin_path, archive_path))
    with tarfile.open(os.path.join(args.publish_dir, archive_name), "x:gz") as tar_fh:
        tar_fh.add(built_bin_path, arcname="electrs")
    logging.info("Done. Waiting forii new repo update.")


