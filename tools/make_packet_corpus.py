"""Write tests/packet/inputs.json and tests/color/inputs.json.

packet: each case is {size, init (hex), ops: [[name, ...args]]} run on a fresh
Packet by `packet_corpus_matches`. Number arguments may be the strings "NaN",
"Infinity", "-Infinity"; p8 takes a decimal string (bigint). Hand-picked edge
cases first, then seeded random op sequences.

usage: python -I tools/make_packet_corpus.py
"""
import json
import random
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "tests"
rng = random.Random(274)

EDGE = [
    # DataView coercions: wrap modulo 2^n, NaN/Infinity → 0, fractions truncate toward zero
    {"size": 64, "ops": [["p1", v] for v in [0, 255, 256, -1, -129, 1.9, -1.9, "NaN", "Infinity", "-Infinity", 2**40 + 7, -(2**53)]]},
    {"size": 64, "ops": [["p2", v] for v in [0, 65535, 65536, -1, 70000.7, "NaN", 2**32 - 1, -32769]]},
    {"size": 64, "ops": [["ip2", 0x1234], ["ip4", -2], ["p3", 0xABCDEF], ["p3", -1], ["p3", 2**24 + 3], ["p3", -(2**31)]]},
    {"size": 64, "ops": [["p4", v] for v in [0, -1, 2**31, 2**32 - 1, 2**32 + 1, -(2**31) - 1, 1e20, "NaN"]]},
    {"size": 32, "ops": [["p8", "-1"], ["p8", "9223372036854775807"], ["p8", "-9223372036854775808"], ["pbool", True], ["pbool", False]]},
    # strings: low byte of each UTF-16 unit, astral → two surrogates
    {"size": 64, "ops": [["pjstr", "hello"], ["pjstr", ""], ["pjstr", "é€Ā"], ["pjstr", "\U0001F600"], ["pjstr", "£"]]},
    # gjstr: terminator, custom terminator, unterminated end drops the last byte, then throws
    {"size": 6, "init": "41420a43440045", "ops": [["gjstr"], ["gjstr", 0], ["gjstr"]]},
    {"size": 2, "init": "4142", "ops": [["gjstr"], ["g1"]]},
    # smart encodings and their range throws
    {"size": 64, "ops": [["psmart", v] for v in [0, 127, 128, 32767]] + [["psmarts", v] for v in [-64, 63, 64, -65, 16383, -16384]]},
    {"size": 8, "ops": [["psmart", 32768]]},
    {"size": 8, "ops": [["psmart", -1]]},
    {"size": 8, "ops": [["psmarts", 16384]]},
    {"size": 32, "init": "00407f8000ffffc000bfff", "ops": [["gsmart"], ["gsmart"], ["gsmart"], ["gsmart"], ["gsmarts"], ["gsmarts"]]},
    # var ints, including values whose top bit is set
    {"size": 64, "ops": [["pVarInt", v] for v in [0, 127, 128, 16383, 16384, 2097151, 2097152, 268435455, 268435456, -1, 2**31, 2**32 + 5]]},
    {"size": 64, "init": "00 7f 8100 ffff7f 8080808000 8fffffff7f".replace(" ", ""), "ops": [["gVarInt"]] * 6},
    # size back-patching
    {"size": 32, "ops": [["p1", 9], ["setpos", 2], ["p4", 7], ["psize1", 4], ["p2", 1], ["psize2", 1], ["p4", 3], ["psize4", 3]]},
    {"size": 8, "ops": [["psize2", 4]]},
    # pdata: copies `length` bytes but advances by `length - offset`
    {"size": 32, "ops": [["pdata", "0102030405", 0, 5], ["pdata", "0102030405", 2, 3], ["p1", 255]]},
    {"size": 4, "ops": [["pdata", "0102030405", 0, 5]]},
    # bit packing; writes past the end are dropped silently, reads throw
    {"size": 8, "ops": [["bitStart"], ["pBit", 1, 1], ["pBit", 3, 5], ["pBit", 12, 4095], ["pBit", 8, 170], ["pBit", 32, -1], ["bitEnd"]]},
    {"size": 2, "ops": [["bitStart"], ["pBit", 16, 65535], ["pBit", 8, 255], ["bitEnd"]]},
    {"size": 8, "init": "a5f00ff0ffff1234", "ops": [["bitStart"], ["gBit", 1], ["gBit", 7], ["gBit", 13], ["gBit", 11], ["gBit", 32], ["bitEnd"], ["gBit", 8]]},
    # reads
    {"size": 32, "init": "ff80807fffff7f8000ffffffff80000000ffffffffffffffff0102", "ops": [["g1"], ["g1b"], ["g2"], ["g2s"], ["ig2"], ["g3"], ["g3s"], ["g4"], ["g4s"], ["g8"], ["gbool"], ["gbool"]]},
    # out-of-range writes and reads: where pos ends up matters (pos++ evaluated before the throw)
    {"size": 3, "ops": [["p1", 1], ["p1", 2], ["p1", 3], ["p1", 4]]},
    {"size": 3, "ops": [["p1", 1], ["p2", 2], ["p2", 3]]},
    {"size": 3, "ops": [["p1", 1], ["p3", 2]]},
    {"size": 4, "ops": [["pjstr", "abcdef"]]},
    {"size": 3, "ops": [["g2"], ["g2"]]},
    {"size": 2, "ops": [["g1"], ["g1"], ["g1"]]},
]

WRITERS = ["p1", "p2", "ip2", "p3", "p4", "ip4", "pVarInt"]
SPECIAL = [0, -1, 1, 127, 128, 255, 256, 32767, 32768, 65535, 65536, 2**31 - 1, 2**31, 2**32 - 1, 2**32, -(2**31), 0.5, -0.5, 123.999, "NaN", "Infinity"]


def num():
    r = rng.random()
    if r < 0.3:
        return rng.choice(SPECIAL)
    if r < 0.6:
        return rng.randint(-(2**33), 2**33)
    if r < 0.8:
        return rng.randint(-300, 300)
    return round(rng.uniform(-1e6, 1e6), 3)


def text():
    pool = "abcxyz 019_-'\"é£€Āÿ中"
    return "".join(rng.choice(pool) for _ in range(rng.randint(0, 8)))


def random_write_case():
    ops = []
    for _ in range(rng.randint(1, 12)):
        r = rng.random()
        if r < 0.6:
            ops.append([rng.choice(WRITERS), num()])
        elif r < 0.7:
            ops.append(["pjstr", text()])
        elif r < 0.8:
            ops.append(["psmart", rng.randint(0, 32767)])
        elif r < 0.9:
            ops.append(["psmarts", rng.randint(-16384, 16383)])
        else:
            ops.append(["p8", str(rng.randint(-(2**63), 2**63 - 1))])
    return {"size": 128, "ops": ops}


def random_bit_case():
    ops = [["setpos", rng.randint(0, 3)], ["bitStart"]]
    for _ in range(rng.randint(1, 10)):
        n = rng.randint(1, 32)
        ops.append(["pBit", n, rng.randint(-(2**31), 2**32 - 1)])
    ops.append(["bitEnd"])
    ops.append(["setpos", 0])
    ops.append(["bitStart"])
    for _ in range(rng.randint(1, 10)):
        ops.append(["gBit", rng.randint(1, 32)])
    init = "".join(f"{rng.randint(0, 255):02x}" for _ in range(rng.randint(0, 48)))
    return {"size": 48, "init": init, "ops": ops}


def random_read_case():
    init = "".join(f"{rng.randint(0, 255):02x}" for _ in range(64))
    # g3 is left out: it panics past the end (documented in packet.rs; no network decoder calls g3)
    readers = ["g1", "g1b", "g2", "g2s", "ig2", "g4", "g4s", "g8", "gsmart", "gsmarts", "gVarInt", "gjstr", "gbool"]
    return {"size": 64, "init": init, "ops": [[rng.choice(readers)] for _ in range(rng.randint(1, 10))]}


def main():
    cases = list(EDGE)
    cases += [random_write_case() for _ in range(300)]
    cases += [random_bit_case() for _ in range(200)]
    cases += [random_read_case() for _ in range(200)]
    (ROOT / "packet").mkdir(parents=True, exist_ok=True)
    (ROOT / "packet/inputs.json").write_text(json.dumps(cases, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")

    color = {
        "rgb24": sorted({0, 0xFFFFFF, 0x35720A, 0x58680B, 0xAAAAAA, 0xFF00FF, 0x010101, 0x808080, 0xFF0000, 0x00FF00, 0x0000FF}
                        | {rng.randint(0, 0xFFFFFF) for _ in range(4000)}),
        "hsl24": [[rng.randint(-300, 300), rng.randint(0, 255), rng.randint(0, 255)] for _ in range(500)],
    }
    (ROOT / "color").mkdir(parents=True, exist_ok=True)
    (ROOT / "color/inputs.json").write_text(json.dumps(color) + "\n", encoding="utf-8")
    print(len(cases), "packet cases;", len(color["rgb24"]), "rgb24 colours")


if __name__ == "__main__":
    main()
