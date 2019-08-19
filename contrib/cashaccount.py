#!/usr/bin/env python3
import hashlib
import sys
import argparse

import client

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--testnet', action='store_true')
    parser.add_argument('--server', action='store')
    parser.add_argument('name', nargs='+')
    parser.add_argument('height', nargs='+')
    args = parser.parse_args()

    if args.testnet:
        port = 60001
    else:
        port = 50001


    addr = 'localhost'
    if args.server:
        addr = args.server

    conn = client.Connection((addr, port))
    print(conn.call('cashaccount.query.name', args.name[0], int(args.height[0])))


if __name__ == '__main__':
    main()
