#!/usr/bin/env bash

# Stop on failure
set -e

if [ -z "$BU_HOME" ]
then
    export BU_HOME=$HOME/BitcoinUnlimited
    echo "INFO: \$BU_HOME not set. Using $BU_HOME."
fi

if [ -z "$ELECRSCASH_PATH" ]
then
    SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" >/dev/null 2>&1 && pwd )"
    export ELECRSCASH_PATH=$SCRIPT_DIR/../target/debug/electrscash
fi

export TEST_RUNNER=$BU_HOME/qa/pull-tester/rpc-tests.py
if [ ! -f "$TEST_RUNNER" ]
then
    # Check if compiled
    echo "ERROR: Did not find test runner at $TEST_RUNNER."
    false # stop script
fi

set -x # echo commands to terminal
(cd $BU_HOME; RUST_BACKTRACE=1 $TEST_RUNNER --electrum.exec="$ELECRSCASH_PATH" --electrum-only)
