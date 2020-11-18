# RPC methods supported

See the [Electrum Cash Protocol specification](https://bitcoincash.network/electrum/)
for "offical" supported RPC methods.

In addition, the following RPC methods are supported.

## blockchain.address.get\_first\_use

See [protocol extras](https://bitcoincash.network/electrum/protocol-methods-extra.html)

## blockchain.scripthash.get\_first\_use

See [protocol extras](https://bitcoincash.network/electrum/protocol-methods-extra.html)

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
