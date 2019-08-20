const ElectrumCli = require('electrum-client')
const BITBOX = require('bitbox-sdk').BITBOX
const assert = require('assert')
const crypto = require('crypto')
const progress = require('cli-progress')
const {stopwatch} = require('durations');

const bitbox = new BITBOX()

let xpub = "tpubD6NzVbkrYhZ4YRDtddy5nLmvdH7Qn6oR7euDpjqnZdniaJDTbaL17Gq86bsVNhKMkYwGvSvhamz5QkouzGJ4e2rkyHWbF5mHGX5Up377zBM";
let num_addresses = 2000;

servers = [
    [60002, 'testnet.bitcoincash.network', 'tls'],
    [50002, 'testnet.imaginary.cash', 'tls'],
];

test_request = async (ecl, scripthashes, method) => {
    process.stdout.write("* " + scripthashes.length
        + "x " + method + " " + ecl['host'])

    promises = [];
    for (let i = 0; i < scripthashes.length; ++i) {
        promises.push(ecl.request(method, [scripthashes[i]]));
    }
    return Promise.all(promises)
}

const main = async () => {
    console.log("Initiating test. Deriving " + num_addresses + " addresses from xpub");
    const derive_bar = new progress.SingleBar({}, progress.Presets.shades_classic);
    derive_bar.start(num_addresses, 0);

    scripthashes = [];

    for (let i = 0; i < num_addresses; ++i) {
        scripthashes.push(toScriptHash(bitbox.Address.fromXPub(xpub, "0/" + i)));
        derive_bar.increment();
    }
    derive_bar.stop()

    let connections = [];
    for (let s of servers) {
        console.log("Connecting to ", s)
        let c = new ElectrumCli(s[0], s[1], s[2])
        await c.connect()
        connections.push(c)
    }


    runtest = async (iterations, test) => {
        let total = new Map()

        for (let i = 0; i < iterations; ++i) {
            const watch = stopwatch()
            for (let c of connections) {
                watch.reset()
                watch.start()
                await test(c)
                let duration = watch.duration()
                if (!total[c['host']]) {
                    total[c['host']] = 0
                }
                total[c['host']] += duration.millis()
                console.log("(" + duration.format() + ")")
            }
        }

        for (let c of connections) {
            console.log("MEAN TIME " + (total[c['host']] / iterations) + "ms for "
                + c['host'])
        }
    }

    await runtest(10, async (c) => {
        await test_request(c, scripthashes, "blockchain.scripthash.subscribe");
    })
    await runtest(10, async (c) => {
        await test_request(c, scripthashes, "blockchain.scripthash.get_history");
    })
    await runtest(10, async (c) => {
        await test_request(c, scripthashes, "blockchain.scripthash.get_balance");
    })


    for (let i = 0; i < connections.length; ++i) {
        await connections[i].close();
    }
    console.log("done");

}

// Convert cashaddr to scripthash
// See https://electrumx.readthedocs.io/en/latest/protocol-basics.html#script-hashes
function toScriptHash(cashaddr) {

    const hash160 = bitbox.Address.cashToHash160(cashaddr);
    let scriptHash = bitbox.Script.fromASM(
        "OP_DUP OP_HASH160 " + hash160 + " OP_EQUALVERIFY OP_CHECKSIG")
    scriptHash = crypto.createHash('sha256').update(scriptHash).digest()
    scriptHash.reverse()
    return scriptHash.toString('hex')
}

main().catch((e) => { console.log("Error:", e) })
