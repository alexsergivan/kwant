#!/usr/bin/env python3
"""Rebuilds guest/fonts/inter-guest.woff2: kwant's Inter latin, cut to U+0020-00FF plus the punctuation the guest page
uses, weights 400-800 (design.md "Guest page budget": <= 30 KB, else the system font). Needs fonttools (pip).

Usage: python3 frontend/guest/tools/subset_font.py frontend/src/assets/fonts/inter-latin.woff2 frontend/guest/fonts/inter-guest.woff2
"""
import sys
from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

src, dst = sys.argv[1], sys.argv[2]
UNICODES = '0020-007E,00A0-00FF,2013,2014,2018-201D,2026,2212,25B2,25BC'
opts = subset.Options()
opts.flavor = None
opts.layout_features = ['kern', 'liga', 'calt', 'tnum']
font = TTFont(src)
sub = subset.Subsetter(opts)
sub.populate(unicodes=subset.parse_unicodes(UNICODES))
sub.subset(font)
font = instancer.instantiateVariableFont(font, {'wght': (400, 800)})
font.flavor = 'woff2'
font.save(dst)
