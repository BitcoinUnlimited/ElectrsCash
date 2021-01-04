# RPC methods supported

See the [Electrum Cash Protocol specification](https://bitcoincash.network/electrum/)
for "offical" supported RPC methods.

## Known deviations

## blockchain.transcation.get

The output for `verbose = true` is implemented in ElectrsCash. The output for
this call is always consistent, regardless of what full node implementation
is used for backend.

If there are breaking changes to this output in the future, this will be done
as part of a major version release of ElectrsCash, meaning first digit of
version number will be increased.

## blockchain.transaction.get_merkle

The `height` parameter is optional with ElectrsCash. If omitted, ElectrsCash
uses its internal index to lookup the transaction height.

## Extensions

In addition, the following RPC methods are supported.

## blockchain.transaction.get\_confirmed\_blockhash

Returns the blockhash of a block the transaction confirmed in. Returns error
if transaction is not confirmed (or does not exist).

Signature: `blockchain.transaction.get_confirmed_blockhash(txid)`

* txid - Transaction ID

### Example result
```
'block_hash': '000000000000000002a04f56505ef459e1edd21fb3725524116fdaedf3a4d0ab',
'block_height': 597843,
```

## blockchain.address.get\_first\_use

See [protocol extras](https://bitcoincash.network/electrum/protocol-methods-extra.html)

## blockchain.scripthash.get\_first\_use

See [protocol extras](https://bitcoincash.network/electrum/protocol-methods-extra.html)

## cashaccount.query.name

Signature: `blockchain.query.name(name, height)`

Returns the transactions registering a cashaccount at blockheight. Note that
height is absolute blockheight and you need to add the cashaccount block
modification value yourself.

The cashaccount block modification value for Bitcoin Cash is 563620.

For example, to lookup dagur#216, call `blockchain.query.name("dagur", 216 +
563620)`

* name - Cash account name
* height - Block height for registration (without cashaccount offset subtracted)

### Example result
Query: `blockchain.query.name('dagur', 563836)`

Result:
```
'blockhash': '000000000000000003c73e50b9de6317c4d2b2ac5f3c1253b01e61a6e329219a',
'height': 563836,
'tx': '0100000001bca903bbc429218234857628b382e8aa8e3bfa74c5b59628ad053284e50bf6ac010000006b4830450221009bbd0a96ef5ef33e09c4fce7fafd2add714ebe05d87a9cb6c826b863d0e99225022039d77b8bd9c8067636e64d6f1aeeeeb8b816bbc875afd04cef9eb299df83b7d64121037a291b1a7f21b03b2a5120434b7a06b61944e0edc1337c76d737d0b5fa1c871fffffffff020000000000000000226a040101010105646167757215018c092ec2cbd842e89432c7c53b54db3a958c83a575f00d00000000001976a914dfdd3e914d73fee85ad40cd71430327f0404c15488ac00000000'
```
