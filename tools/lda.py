#!/usr/bin/env python3
"""Parse DEC absolute-loader (.LDA) images.

Record format, confirmed empirically against the Crystal Castles archive:

    01 00 <count:16le> <addr:16le> <data...> <checksum:8>

`count` spans the 6-byte header plus the data and excludes the checksum byte.
The checksum is chosen so the 8-bit sum of the whole record including the
checksum is zero. A record with count == 6 carries no data and is the
end/transfer-address record; an odd transfer address is DEC's "do not start"
convention (Crystal Castles uses 0x0001).

Files are VAX block-padded with NUL bytes to a 512-byte multiple; trailing NUL
padding after the final record is expected and is not an error.

No dependencies beyond the standard library.
"""

import sys
import argparse


class LdaError(Exception):
    pass


class Record:
    __slots__ = ("offset", "addr", "data", "checksum", "checksum_ok")

    def __init__(self, offset, addr, data, checksum, checksum_ok):
        self.offset = offset
        self.addr = addr
        self.data = data
        self.checksum = checksum
        self.checksum_ok = checksum_ok

    @property
    def is_terminator(self):
        return len(self.data) == 0


def parse(blob):
    """Return (records, trailing_padding_bytes). Raises LdaError on malformed input."""
    records = []
    off = 0
    n = len(blob)
    while off < n:
        if blob[off] != 0x01:
            # Expect only NUL block padding from here to end of file.
            tail = blob[off:]
            if tail.strip(b"\x00"):
                raise LdaError(
                    "unexpected byte %#04x at offset %#x; not a record marker and "
                    "not NUL padding" % (blob[off], off)
                )
            return records, len(tail)
        if off + 6 > n:
            raise LdaError("truncated record header at offset %#x" % off)
        if blob[off + 1] != 0x00:
            raise LdaError(
                "record at %#x has second marker byte %#04x, expected 0x00"
                % (off, blob[off + 1])
            )
        count = blob[off + 2] | (blob[off + 3] << 8)
        addr = blob[off + 4] | (blob[off + 5] << 8)
        if count < 6:
            raise LdaError("record at %#x declares count %d, minimum is 6" % (off, count))
        if off + count >= n:
            raise LdaError(
                "record at %#x declares count %d, overrunning end of file" % (off, count)
            )
        body = blob[off : off + count]
        checksum = blob[off + count]
        ok = ((sum(body) + checksum) & 0xFF) == 0
        records.append(Record(off, addr, body[6:], checksum, ok))
        off += count + 1
    return records, 0


def ranges(records):
    """Coalesce loaded regions into sorted (start, end_exclusive) spans."""
    spans = sorted(
        (r.addr, r.addr + len(r.data)) for r in records if not r.is_terminator
    )
    merged = []
    for start, end in spans:
        if merged and start <= merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], end)
        else:
            merged.append([start, end])
    return [(s, e) for s, e in merged]


def image(records):
    """Build a sparse address->byte map, detecting overlapping writes."""
    mem = {}
    overlaps = []
    for r in records:
        for i, b in enumerate(r.data):
            a = r.addr + i
            if a in mem and mem[a] != b:
                overlaps.append(a)
            mem[a] = b
    return mem, overlaps


def report(path, extract=None):
    blob = open(path, "rb").read()
    try:
        records, padding = parse(blob)
    except LdaError as e:
        print("%s: MALFORMED — %s" % (path, e))
        return False

    data_recs = [r for r in records if not r.is_terminator]
    term = [r for r in records if r.is_terminator]
    bad = [r for r in records if not r.checksum_ok]
    mem, overlaps = image(records)
    spans = ranges(records)

    print("%s" % path)
    print("  file size          : %d bytes (%d NUL padding after final record)"
          % (len(blob), padding))
    print("  records            : %d (%d data, %d terminator)"
          % (len(records), len(data_recs), len(term)))
    print("  checksum           : %d/%d pass%s"
          % (len(records) - len(bad), len(records),
             "" if not bad else "  <-- FAILURES"))
    for r in bad:
        print("      FAIL record at offset %#x, addr %#06x, checksum %#04x"
              % (r.offset, r.addr, r.checksum))
    print("  loaded bytes       : %d" % len(mem))
    print("  address ranges     : %s"
          % ", ".join("%04X-%04X" % (s, e - 1) for s, e in spans))
    if overlaps:
        print("  overlapping writes : %d addresses (first %#06x)"
              % (len(overlaps), overlaps[0]))
    for r in term:
        print("  transfer address   : %#06x (%s)"
              % (r.addr, "start" if r.addr % 2 == 0 else "do not start"))
    if mem:
        lo = min(mem)
        first = bytes(mem.get(lo + i, 0) for i in range(16))
        printable = "".join(chr(b) if 32 <= b < 127 else "." for b in first)
        print("  first 16 at %04X   : %s  |%s|"
              % (lo, " ".join("%02x" % b for b in first), printable))

    if extract:
        if not mem:
            print("  nothing to extract")
            return not bad
        lo, hi = min(mem), max(mem)
        out = bytes(mem.get(a, 0) for a in range(lo, hi + 1))
        with open(extract, "wb") as f:
            f.write(out)
        gaps = (hi - lo + 1) - len(mem)
        print("  extracted          : %d bytes covering %04X-%04X -> %s%s"
              % (len(out), lo, hi, extract,
                 "" if not gaps else "  (%d unwritten bytes zero-filled)" % gaps))

    return not bad


def main():
    ap = argparse.ArgumentParser(description="Parse DEC absolute-loader (.LDA) images.")
    ap.add_argument("files", nargs="+")
    ap.add_argument("--extract", metavar="OUTFILE",
                    help="write a flat binary of the loaded image (single input file only)")
    args = ap.parse_args()

    if args.extract and len(args.files) != 1:
        ap.error("--extract takes a single input file")

    ok = True
    for i, path in enumerate(args.files):
        if i:
            print()
        ok &= report(path, args.extract)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
