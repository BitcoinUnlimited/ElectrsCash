#!/usr/bin/env python3
# Copyright (c) 2019 The Bitcoin Unlimited developers
# Distributed under the MIT software license, see the accompanying
# file COPYING or http://www.opensource.org/licenses/mit-license.php.

"""
Attempt two builds and check if the resulting build is equal.
"""
import argparse
import logging
import os
import shutil
import subprocess
import sys
import tempfile
from utilbuild import cargo_run

FAKETIME_TIMESTAMP = '12am'

parser = argparse.ArgumentParser()
parser.add_argument('--skip-build', help='Skip (re)building electrscash',
        action = "store_true")
parser.add_argument('--verbose', help='Sets log level to DEBUG',
        action = "store_true")
parser.add_argument('--build1-dir', default="/tmp/ec-build1",
    help="Directory of #1 build (warning: gets deleted if it exists)")
parser.add_argument('--build2-dir', default="/tmp/ec-build2",
    help="Directory of #2 build (warning: gets deleted if it exists)")
args = parser.parse_args()

level = logging.DEBUG if args.verbose else logging.INFO
logging.basicConfig(format = '%(levelname)s: %(message)s',
    level=level,
    stream=sys.stdout)

def file_digest(path):
    import hashlib
    hasher = hashlib.sha256()
    with open(str(path), 'rb') as fh:
        chunk = fh.read(hasher.block_size)
        while chunk:
            hasher.update(chunk)
            chunk = fh.read(hasher.block_size)
    return hasher.hexdigest()

def hash_artifacts(target_dir, file_patterns):
    from pathlib import Path
    artifacts = [ ]
    for p in file_patterns:
        artifacts.extend(list(Path(target_dir).glob("**/" + p)))

    hashed = { }
    prefix_ignored = len(target_dir)
    for f in artifacts:
        key = str(f)[prefix_ignored:]
        hashed[key] = file_digest(f)
    return hashed



def rmdir(path):
    try:
        logging.debug("Clearing " + path)
        shutil.rmtree(path)
    except FileNotFoundError as e:
        logging.debug(str(e))
    assert not os.path.exists(path)

def extract_lib(path):
    dir_path, filename = os.path.split(path)
    logging.debug("\textracting %s" % path)
    result = subprocess.run(
        ["ar", "x", filename], check = True, cwd = dir_path)
    assert result.stderr is None
    assert result.stdout is None

def check_obj_files(lib, build1_dir, build2_dir):
    logging.info("\tInvestigating object files of %s" % lib)
    lib1_dir = tempfile.mkdtemp()
    lib2_dir = tempfile.mkdtemp()
    cpy1 = os.path.join(lib1_dir, "lib.rlib")
    cpy2 = os.path.join(lib2_dir, "lib.rlib")

    if lib[0] == "/":
        # os.path.join doesn't like / prefix, remove it
        lib = lib[1:]

    shutil.copyfile(os.path.join(build1_dir, lib), cpy1)
    shutil.copyfile(os.path.join(build2_dir, lib), cpy2)


    extract_lib(cpy1)
    extract_lib(cpy2)

    a1 = hash_artifacts(lib1_dir, ["*.o"])
    a2 = hash_artifacts(lib2_dir, ["*.o"])

    if len(a1) != len(a2):
        raise Exception("%s has %d artifacts, while %s has %d artifacts",
                build1_dir, len(a2), build2_dir, len(a2))

    if len(a1) == 0:
        raise Exception("No object files found in %s" % lib)

    ok = True
    for artifact in sorted(a1):
        if a1[artifact] == a2[artifact]:
            logging.debug("\t%s: OK!" % artifact)
            continue

        ok = False
        logging.info("%s %s %s: FAILED!" % (artifact, a1[artifact], a2[artifact]))

    return ok, lib1_dir, lib2_dir

def build(build1_dir, build2_dir):
    rmdir(build1_dir)
    rmdir(build2_dir)

    def clear_cache():
        # Force rebuild of dependencies
        from pathlib import Path
        home = str(Path.home())
        rmdir(os.path.join(home, ".cargo", ".git"))
        rmdir(os.path.join(home, ".cargo", ".registry"))

    clear_cache()
    cargo_run(["build", "--release", "--target=x86_64-unknown-linux-gnu", "--target-dir=" + build1_dir], logging, faketime = FAKETIME_TIMESTAMP)
    clear_cache()
    cargo_run(["build", "--release", "--target=x86_64-unknown-linux-gnu", "--target-dir=" + build2_dir], logging, faketime = FAKETIME_TIMESTAMP)

def check_artifacts(build1_dir, build2_dir):
    a1 = hash_artifacts(build1_dir, ["*.rlib", "*.a", "*.so"])
    a2 = hash_artifacts(build2_dir, ["*.rlib", "*.a", "*.so"])

    if len(a1) != len(a2):
        logging.error("%s has %d artifacts, while %s has %d artifacts",
            build1_dir, len(a2), build2_dir, len(a2))
        return False

    if len(a1) == 0:
        logging.error("No artifacts found")
        return False

    ok = True
    for artifact in sorted(a1):
        if a1[artifact] == a2[artifact]:
            continue

        logging.warning("MISMATCH in %s" % artifact)
        logging.warning("\t%s: %s" % (build1_dir, a1[artifact]))
        logging.warning("\t%s: %s" % (build2_dir, a1[artifact]))

        o_files_ok, tmp1, tmp2 = check_obj_files(artifact, build1_dir, build2_dir)
        if o_files_ok:
            # Empirically we've seen this may not be an issue as long as object
            # files match.
            # To investigate other files, try: diff -qrN build1_dir build2_dir
            logging.debug("Object files OK!")

        ok = ok and o_files_ok

    return ok

def check_electrscash_bin(build1_dir, build2_dir):
    a1 = hash_artifacts(build1_dir, ["electrscash"])
    a2 = hash_artifacts(build2_dir, ["electrscash"])
    assert(len(a1) == 1)
    assert(len(a2) == 1)
    electrscash_bin = list(a1.keys())[0]

    ok = a1[electrscash_bin] == a2[electrscash_bin]
    if ok:
        logging.debug("%s OK!" % electrscash_bin)
    else:
        logging.error("%s does not match" % electrscash_bin)
        logging.error("\t%s: %s" % (build1_dir, a1[electrscash_bin]))
        logging.error("\t%s: %s" % (build2_dir, a1[electrscash_bin]))
    return ok

if not args.skip_build:
    build(args.build1_dir, args.build2_dir)

ok = check_artifacts(args.build1_dir, args.build2_dir)
ok = ok and check_electrscash_bin(args.build1_dir, args.build2_dir)

if ok:
    logging.info("SUCCESS - builds are deterministic!")
else:
    logging.error("FAILED CHECK")
sys.exit(1 - int(ok))
