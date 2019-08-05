#!/usr/bin/env python3
# Copyright (c) 2019 The Bitcoin Unlimited developers
# Distributed under the MIT software license, see the accompanying
# file COPYING or http://www.opensource.org/licenses/mit-license.php.

"""
Tool to compare two binary builds, used for investigating differences when/if
deterministic build breaks.
"""

from binascii import *
import argparse
import pdb
import logging
import sys

# pip install elftools
from elftools.elf.elffile import *
from elftools.elf.relocation import *

parser = argparse.ArgumentParser()
parser.add_argument("bin1", metavar='bin1', type=str, help="path to first elf")
parser.add_argument("bin2", metavar='bin2', type=str, help="path to second elf")
args = parser.parse_args()

logging.basicConfig(format = '%(asctime)s.%(levelname)s: %(message)s',
    level=logging.DEBUG,
    stream=sys.stdout)

f1 = open(args.bin1, 'rb')
f2 = open(args.bin2, 'rb')

elf1 = ELFFile(f1)
elf2 = ELFFile(f2)

for (section1,section2) in zip(elf1.iter_sections(),elf2.iter_sections()):
    print(str(section1))
    print(str(section2))
    if (section1.data() == section2.data()):
        continue
    print(section1.name)
    d = section1.data()
    dcut = d[0:1000] if len(d) > 1000 else d
    print(hexlify(dcut))
    print(section2.name)
    d = section2.data()
    dcut = d[0:1000] if len(d) > 1000 else d
    print(hexlify(dcut))
    pdb.set_trace()
    print("MISCOMPARE!")
