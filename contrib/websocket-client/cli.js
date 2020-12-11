const { ElectrumClient, ElectrumTransport } = require('electrum-cash');

function guess_datatype (args) {
    return args.map((a) => {
        if (a.toLowerCase() == "true") {
            return true;
        }
        if (a.toLowerCase() == "false") {
            return false;
        }
        if (/^\d+$/.test(a)) {
            return parseInt(a);
        }
        return a;
    });
}


(async () => {
    const electrum = new ElectrumClient('Test client',
        '1.4.1', '127.0.0.1', ElectrumTransport.WS.Port, ElectrumTransport.WS.Scheme);

    await electrum.connect();

    const args = guess_datatype(process.argv.slice(2));
    console.log(await electrum.request(...args));

    electrum.disconnect();

})().catch((e) => { console.error(e) });
