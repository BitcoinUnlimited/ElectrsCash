```
Electrum Protocol Improvement Proposal
Title: Cash Account lookup RPC methods
Author: Dagur Valberg Johannsson <dagurval@pvv.ntnu.no>
Status: Draft
```

# Abstract

Cash Account is a popular naming system for sharing payment information on the Bitcoin Cash network. We describe a method to allow electrum clients to query for cash account payment information from electrum servers.

# Motivation

As currently implemented in existing wallets on Bitcoin Cash, including SPV wallets running on the Electrum network, the wallets query an [external lookup server](https://gitlab.com/cash-accounts/lookup-server) for payment information. Rather than relying on external lookup servers, we want to provide a method of querying this information directly from the electrum network.

Supporting Cash Account lookups via electrum servers can:

* **Improve reliablity**: The client can use existing server connection for lookup, rather than depending on  additional servers.
* **Improve privacy**: The lookup query exposes that the client is interested in specific payment information to the server. Having to request this query from fewer entities improves privacy.
* **Improve availablity**: There are only a few known public cashaccount lookup servers available. Having electrum servers support cash account lookups improves availability.
* **Improve SPV security**: One method of mitigating the [Index Collusion attack](https://gitlab.com/cash-accounts/specification/blob/master/SPECIFICATION.md) is to cross-referencing lookup data from multiple servers. More independent servers improve this mitigation method.

# Specification

We introduce a new protocol method `blockchain.cashaccount.lookup`. This method is does not depend on blockchain-specific attributes such as the *Block Modification Value* (see Cash Account specification) and is thus blockchain agnostic.

## blockchain.cashaccount.lookup

Return a sorted list of transactions that match cashaccount name and blockheight from given offset, number of results and the hash of the block at given blockheight. The full transactions are returned for the client to decode the payment infromation and verify the transaction itself.

The transactions are sorted by their little endian hash value (same as canonical transaction order as inlemented on the Bitcoin Cash network).

On zero matches, the list of transactions is empty. Otherwise the number of transactions returned must be at least one, while the upper limit of returned transactions is undefined.

The offset is zero-indexed offset into the sorted list of transactions.

### Signature

`blockchain.cashaccount.lookup(cashaccount_name, blockheight, offset = 0)`

#### *cashaccount_name*

The cashaccount name is the string chosen by the user at registration time and can be at most 99 characters long.

The string must adhere to a strict Regular Expression of `/^[a-zA-Z0-9_]{1,99}$/`.

#### *blockheight*

The height of the block that mined the registration transaction.

A note on cashaccount addresses: The blockheight is the integer after the `#` separator of a cashaccount address, added with a *Block Modification Value*. The block modification value is blockchain-dependent. The Block Modification Value of the Bitcoin Cash network is *563620*.

Example: To query the address `Jonathan#100` on Bitcoin Cash, the blockheight is `563620 + 100 = 563720` the parameters passed should be `cashaccount_name="Jonathan", blockheight=563720`.

#### *offset*

Zero-indexed offset to the first result of the sorted list of matched transactions. Defaults to 0.

### Example result

Result for cashaccount `Jonathan#100`, which translates to call `blockchain.cashaccount.lookup("Jonathan", "563720")`

```
{
    blockhash: '000000000000000002abbeff5f6fb22a0b3b5c2685c6ef4ed2d2257ed54e9dcb',
    results: 1,
    transactions: [
     '01000000017cc04d29109cb43a0bfade3993b5840b6f68f22e09d4806f26fe9e7d772fc72f010000006a47304402207b0da3150bf9a44a8fae7333f4d5b03ba1297dd21c8641880efea9eaa9e1e89d022028d2a8d840771d4a87c84f0b217d02f17c3f57832fe0c37ac27b5afc27004fbe41210355f64f0ed04944eb477b33dcb46bb45453b8988bba1862698abe7343c6f0e2c6ffffffff020000000000000000256a0401010101084a6f6e617468616e1501ebdeb6430f3d16a9c6758d6c0d7a400c8e6bbee4c00c1600000000001976a914efd03e75f2aedb19261b39a6c8361c7bccd9f4f088ac00000000']
}
```

### Error conditions

* If the `blockheight` parameter is less than the *cash account activation blockheight* an error is returned. The cash account activation blockheight is *563720* on the Bitcoin Cash network.
* If the `blockheight` is larger than the height of the current blockchain tip an error is returned.
* If the `cashaccount_name` is empty, or longer than 99 characters, or does not adhere to the regular expression as defined above, an error is returned.
* If the `offset` is a negative value, or larger than `results - 1`, an error is returned.

# Recommendations

To announce cashaccount query support to the electrum clients, it is recommended that the server adds the key-value pair `"cashaccount": ["1.0"]` to the response of the `servers.features`method.

The `1.0` is a version value that represents that the server supports this specification. Additional values may be present as well, representing future cashaccount protocol changes. If the client does not understand these version numbers, they should be ignored.

It is recommended that the server responds with as many transactions as possible for a query, within reasonable deny-of-service attack limits. Note that there is no limit on how many transactions in a block can register the same name, besides the block size limit itself.

# Backward compatibility

This protocol extension can be discovered through the `servers.features` RPC method. This improvement implements the new method `blockchain.cashaccount.lookup`. The new method does not collide with any existing method. As such, it is fully backward compatible with existing protocol.

# Implementations

* https://github.com/BitcoinUnlimited/ElectrsCash/pull/2 - Implementation of an earlier draft of this proposal

# References

* [Cash Account Specification](https://gitlab.com/cash-accounts/specification/blob/master/SPECIFICATION.md)

# Copyright

This document is licensed under the BSD 2-clause license.



