"""Strip SteamStub Variant 2.1 (x86) from SR2_pc.exe for static analysis.

Independent reimplementation of the algorithm documented by atom0s' Steamless
(https://github.com/atom0s/Steamless, Variant21.x86). AES comes from Windows CNG
(bcrypt.dll) via ctypes, so no third-party packages are needed.

Output keeps the original section layout (.bind is left in place); only the
decrypted code bytes and AddressOfEntryPoint are patched. That keeps file
offsets == the original's, which is what we want for analysis.

usage: python unsteamstub21.py <in.exe> <out.exe>
"""
import ctypes
import re
import struct
import sys
from ctypes import wintypes


def u32(b, o):
    return struct.unpack_from("<I", b, o)[0]


class PE:
    def __init__(self, data):
        self.d = data
        self.pe = u32(data, 0x3C)
        self.nsec = struct.unpack_from("<H", data, self.pe + 6)[0]
        optsz = struct.unpack_from("<H", data, self.pe + 20)[0]
        self.opt = self.pe + 24
        self.ep = u32(data, self.opt + 16)
        self.base = u32(data, self.opt + 28)
        self.secs = []
        s = self.opt + optsz
        for i in range(self.nsec):
            name, vs, va, rs, ro = struct.unpack_from("<8sIIII", data, s + i * 40)
            self.secs.append(dict(name=name.rstrip(b"\0").decode(), vs=vs, va=va, rs=rs, ro=ro))

    def off(self, rva):
        for s in self.secs:
            if s["va"] <= rva < s["va"] + max(s["vs"], s["rs"]):
                return rva - s["va"] + s["ro"]
        raise ValueError(hex(rva))

    def owner(self, rva):
        for s in self.secs:
            if s["va"] <= rva < s["va"] + max(s["vs"], s["rs"]):
                return s
        raise ValueError(hex(rva))

    def section(self, name):
        return next(s for s in self.secs if s["name"] == name)


def steam_xor(buf, key=0):
    buf = bytearray(buf)
    o = 0
    if key == 0:
        key = u32(buf, 0)
        o = 4
    for x in range(o, len(buf) - 3, 4):
        v = u32(buf, x)
        struct.pack_into("<I", buf, x, v ^ key)
        key = v
    return bytes(buf), key


def xtea_decrypt_block(keys, v1, v2, n=32):
    delta, m = 0x9E3779B9, 0xFFFFFFFF
    s = (delta * n) & m
    for _ in range(n):
        v2 = (v2 - ((((v1 << 4) ^ (v1 >> 5)) + v1) ^ (s + keys[(s >> 11) & 3]))) & m
        s = (s - delta) & m
        v1 = (v1 - ((((v2 << 4) ^ (v2 >> 5)) + v2) ^ (s + keys[s & 3]))) & m
    return v1, v2


def drmp_decrypt(buf, keys):
    buf = bytearray(buf)
    p1 = p2 = 0x55555555
    for x in range(0, len(buf) - 7, 8):
        d1, d2 = struct.unpack_from("<II", buf, x)
        r1, r2 = xtea_decrypt_block(keys, d1, d2)
        struct.pack_into("<II", buf, x, r1 ^ p1, r2 ^ p2)
        p1, p2 = d1, d2
    return bytes(buf)


def find_pattern(data, pat):
    rx = b"".join(b"." if t == "??" else re.escape(bytes([int(t, 16)])) for t in pat.split())
    m = re.search(rx, data, re.S)
    return m.start() if m else -1


# --- Windows CNG AES -------------------------------------------------------
_bc = ctypes.WinDLL("bcrypt")
_H = ctypes.c_void_p


def _ok(st):
    if st != 0:
        raise OSError("bcrypt status 0x%08x" % (st & 0xFFFFFFFF))


def aes(key, data, mode, iv=None):
    alg, hkey = _H(), _H()
    _ok(_bc.BCryptOpenAlgorithmProvider(ctypes.byref(alg), "AES", None, 0))
    try:
        m = "ChainingModeECB" if mode == "ecb" else "ChainingModeCBC"
        mb = ctypes.create_unicode_buffer(m)
        _ok(_bc.BCryptSetProperty(alg, "ChainingMode", mb, ctypes.sizeof(mb), 0))
        kb = ctypes.create_string_buffer(key, len(key))
        _ok(_bc.BCryptGenerateSymmetricKey(alg, ctypes.byref(hkey), None, 0, kb, len(key), 0))
        try:
            inb = ctypes.create_string_buffer(data, len(data))
            outb = ctypes.create_string_buffer(len(data))
            ivb = ctypes.create_string_buffer(iv, len(iv)) if iv else None
            got = wintypes.ULONG()
            _ok(_bc.BCryptDecrypt(hkey, inb, len(data), None, ivb, len(iv) if iv else 0,
                                  outb, len(data), ctypes.byref(got), 0))
            return outb.raw[: got.value]
        finally:
            _bc.BCryptDestroyKey(hkey)
    finally:
        _bc.BCryptCloseAlgorithmProvider(alg, 0)


# --- unpack -----------------------------------------------------------------
def decode_entry(pe):
    """Mirror Steamless' scan: 1st `mov [mem], imm32` = header VA, 2nd = xor key,
    `mov reg32, imm32` = dword count. Hand-decoded for the encodings SteamStub uses."""
    d, o = pe.d, pe.off(pe.ep)
    assert u32(d, o - 4) == 0xC0DEC0DE, "missing 0xC0DEC0DE marker"
    hdr_va = size = key = 0
    end = o + 4096
    while o < end and not (hdr_va and size and key):
        b = d[o]
        if b == 0xC7 and d[o + 1] == 0x85:  # mov dword [ebp+disp32], imm32
            imm = u32(d, o + 6)
            if not hdr_va:
                hdr_va = imm
            else:
                key = imm
            o += 10
        elif 0xB8 <= b <= 0xBF:  # mov r32, imm32
            size = u32(d, o + 1) * 4
            o += 5
        elif b in (0x8B, 0x89, 0x8D) and (d[o + 1] & 0xC7) == 0x85:  # r/m [ebp+disp32]
            o += 6
        elif b in (0x8B, 0x89) and d[o + 1] >= 0xC0:  # mov r32, r32
            o += 2
        elif b == 0x81 and d[o + 1] == 0xEC:  # sub esp, imm32
            o += 6
        elif b == 0xF3 and d[o + 1] == 0xA5:  # rep movsd
            o += 2
        elif 0x50 <= b <= 0x57:  # push r32
            o += 1
        else:
            raise ValueError("unhandled opcode %02x at file off %x" % (b, o))
    return hdr_va - pe.base, size, key


def main(src, dst):
    data = open(src, "rb").read()
    pe = PE(data)
    hdr_rva, hdr_size, hdr_key = decode_entry(pe)
    print("header rva %#x size %#x key %#x" % (hdr_rva, hdr_size, hdr_key))
    assert hdr_size // 4 != 0xD0, "D0 header variant not handled"

    ho = pe.off(hdr_rva)
    hdr, key = steam_xor(data[ho:ho + hdr_size], hdr_key)
    f = struct.unpack_from("<13I8s3I", hdr, 0)
    payload_va, payload_size, appid = f[9], f[10], f[11]
    drmp_va_off, drmp_size_off, xtea_off = f[14], f[15], f[16]
    print("appid %d payload va %#x size %#x" % (appid, payload_va, payload_size))

    po = pe.off(payload_va - pe.base)
    payload, key = steam_xor(data[po:po + payload_size], key)

    drmp_va = u32(payload, drmp_va_off)
    drmp_size = u32(payload, drmp_size_off)
    keys = [u32(payload, xtea_off + 4 * i) for i in range((len(payload) - xtea_off) // 4)]
    do = pe.off(drmp_va - pe.base)
    drmp = drmp_decrypt(data[do:do + drmp_size], keys)
    print("SteamDRMP.dll %#x bytes, starts %r" % (drmp_size, drmp[:2]))

    fallback = False
    k = find_pattern(drmp, "8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8D ?? ?? ?? ?? ?? 05")
    if k < 0:
        k = find_pattern(drmp, "8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8B")
    if k < 0:
        k = find_pattern(drmp, "8B ?? ?? ?? ?? ?? 89 ?? ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? A3 ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? A3 ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? A3 ?? ?? ?? ?? 8B")
        fallback = True
    assert k >= 0, "offset block not found in SteamDRMP.dll"
    blk = drmp[k:k + 1024]
    i32 = lambda o: struct.unpack_from("<i", blk, o)[0]
    flags_o, oep_o = i32(2), i32(25 if fallback else 26)
    cva_o, csz_o = i32(36 if fallback else 38), i32(47 if fallback else 50)
    aeskey_o, aesiv_o = i32(61 if fallback else 62), i32(72 if fallback else 67)
    stolen_o = aesiv_o + 16

    flags = u32(payload, flags_o)
    oep = u32(payload, oep_o)
    code_va = u32(payload, cva_o)
    enc_size = u32(payload, csz_o)
    print("flags %#x oep %#x code va %#x enc size %#x" % (flags, oep, code_va, enc_size))

    sec = pe.owner(code_va - pe.base)
    out = bytearray(data)
    if not flags & 0x04:  # NoEncryption not set
        aeskey = payload[aeskey_o:aeskey_o + 32]
        iv = aes(aeskey, payload[aesiv_o:aesiv_o + 16], "ecb")
        stolen = payload[stolen_o:stolen_o + 16]
        so = pe.off(sec["va"])
        blob = stolen + data[so:so + enc_size]
        blob = blob[: len(blob) // 16 * 16]
        plain = aes(aeskey, blob, "cbc", iv)
        n = min(enc_size, len(plain))
        out[so:so + n] = plain[:n]
        print("decrypted %#x bytes of %s" % (n, sec["name"]))
    struct.pack_into("<I", out, pe.opt + 16, oep - pe.base)  # AddressOfEntryPoint
    struct.pack_into("<I", out, pe.opt + 64, 0)  # CheckSum
    open(dst, "wb").write(out)
    print("wrote", dst, "OEP rva %#x" % (oep - pe.base))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
