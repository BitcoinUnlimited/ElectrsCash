# RPC methods supported

See the [Electrum Cash Protocol specification](https://bitcoincash.network/electrum/)
for "offical" supported RPC methods.

## Extensions

In addition to the above supported RPC methods, ElectrsCash implements the following extensions.

### blockchain.address.get\_first\_use

See [protocol extras](https://bitcoincash.network/electrum/protocol-methods-extra.html)

### blockchain.scripthash.get\_first\_use

See [protocol extras](https://bitcoincash.network/electrum/protocol-methods-extra.html)

### blockchain.transaction.get\_merkle

The `height` parameter is optional with ElectrsCash. If omitted, ElectrsCash
uses its internal index to lookup the transaction height.

### blockchain.transaction.get\_confirmed\_blockhash

Returns the blockhash of a block the transaction confirmed in. Returns error
if transaction is not confirmed (or does not exist).

Signature: `blockchain.transaction.get_confirmed_blockhash(txid)`

* `txid` - Transaction ID

#### Example result
```
'block_hash': '000000000000000002a04f56505ef459e1edd21fb3725524116fdaedf3a4d0ab',
'block_height': 597843,
```

### blockchain.utxo.get

Returns data on a specified output of specific transaction. Returns error
if transaction or output does not exist.

If the output is spent, information about the spender is provided. This allows
a SPV client to call `blockchain.transaction.get_merkle` to generate a merkle
branch, proving that it is spent.

Signature: `blockchain.utxo.get(tx_hash, output_index)`

* `tx_hash` - Transaction ID
* `output_index` - The vout position in the transaction.

#### Result

A dictionary with the following keys:

* `state` - State of the utxo. A string that is "spent" or "unspent".

* `height`- The height the utxo was confirmed in. If it is unconfirmed, the
   value is 0 if all inputs are confirmed, and -1 otherwise.

* `value` - The output’s value in minimum coin units (satoshis).

* `scripthash` - The scriphash of the output scriptPubKey.

* `spent` - The transaction spending the utxo with the following keys:

    * `tx_pos` - The zero-based index of the input in the transaction’s list of inputs. Null if utxo is unspent.

    * `tx_hash` - The transaction ID. Null if utxo is unspent.

    * `height`- The height the transaction was confirmed in. If it is unconfirmed, the
       value is 0 if all inputs are confirmed, and -1 otherwise. Null if utxo is unspent.

#### Example result
```
{
    "amount": 4999999000,
    "height": 100000,
    "scripthash": "2e6d15f1a36288b55d5cb14d21f00324cbf767b459dc37e5054e383e434e0b16",
    "spent": {
        "height": -1,
        "tx_hash": "90adba10cdb91546b9c17e93ee300fe7940c6c3dda80f83bb791df5895d83aff",
        "tx_pos": 0
    },
    "status": "spent"
},
```

### cashaccount.query.name

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

## Notable differences

### blockchain.transcation.get

The output for `verbose = true` is implemented in ElectrsCash. The output for
this call is always consistent, regardless of what full node implementation
is used for backend.

If there are breaking changes to this output in the future, this will be done
as part of a major version release of ElectrsCash, meaning first digit of
version number will be increased.
