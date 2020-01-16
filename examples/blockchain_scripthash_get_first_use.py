#!/usr/bin/env python3
import bitcoincash

from bitcoincash.electrum import Electrum
from bitcoincash.electrum.svr_info import ServerInfo
from bitcoincash.core import CBlockHeader, x
from bitcoincash.wallet import CBitcoinAddress
import asyncio

bitcoincash.SelectParams("testnet")
scripthash = CBitcoinAddress("bchtest:qq2ckhgcz4fvna8jvlqdu692ujtrqsue8yarpm648v").to_scriptHash()

async def electrum_stuff():
    cli = Electrum()

    await cli.connect(ServerInfo("localhost", hostname="localhost", ports=["t60001"]))

    print(await cli.RPC('blockchain.scripthash.get_first_use', scripthash))

    await cli.close()

loop = asyncio.get_event_loop()
loop.run_until_complete(electrum_stuff())
loop.close()
