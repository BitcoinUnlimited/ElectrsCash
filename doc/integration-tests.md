# Integration testing

Integration tests are included in the [Bitcoin Unlimited test set](https://github.com/BitcoinUnlimited/BitcoinUnlimited/tree/dev/qa/rpc-tests). Look for tests prefixed with `electrum_`.

To run the tests, you need to [download and build Bitcoin Unlimited](https://github.com/BitcoinUnlimited/BitcoinUnlimited/blob/release/doc/build-unix.md).

After you can run the tests directly by calling `./qa/rpc-tests/electrum_basics.py` from the Bitcoin Unlimited root directory.

The tests assume ElectrsCash binary is located at `<BitcoinUnlimited/src/electrscash>`. To have it run your build, you can either symlink it there, or you can pass the parameter `--electrum.exec`, such as `./qa/rpc-tests/electrum_basics.py --electrum.exec=/home/user/ElectrsCash/target/debug/electrscash`.
