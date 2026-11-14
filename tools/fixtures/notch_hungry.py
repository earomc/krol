# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Writes parity/fixtures/playerdata/notch_hungry.dat: Notch in survival mode at half
# hunger, holding two bread, a mushroom stew and a cooked salmon. Only the fields that
# matter are written; the server fills in the rest like for any partial player file.
#
# Run: python tools/fixtures/notch_hungry.py

import gzip
import struct


def name(n):
    b = n.encode()
    return struct.pack('>H', len(b)) + b


def tag(t, n, payload):
    return bytes([t]) + name(n) + payload


def byte(n, v): return tag(1, n, struct.pack('>b', v))
def short(n, v): return tag(2, n, struct.pack('>h', v))
def int_(n, v): return tag(3, n, struct.pack('>i', v))
def float_(n, v): return tag(5, n, struct.pack('>f', v))
def string(n, v): return tag(8, n, name(v))
def compound_payload(*tags): return b''.join(tags) + b'\x00'


def list_(n, elem_type, payloads):
    return tag(9, n, bytes([elem_type]) + struct.pack('>i', len(payloads)) + b''.join(payloads))


def double_list(n, values):
    return list_(n, 6, [struct.pack('>d', v) for v in values])


def float_list(n, values):
    return list_(n, 5, [struct.pack('>f', v) for v in values])


def item(slot, item_id, count, damage):
    return compound_payload(byte('Count', count), short('Damage', damage), byte('Slot', slot),
                            string('id', item_id))


root = compound_payload(
    int_('playerGameType', 0),
    int_('Dimension', 0),
    double_list('Pos', [-473.5, 4.0, -440.5]),
    float_list('Rotation', [0.0, 0.0]),
    byte('OnGround', 1),
    int_('foodLevel', 10),
    float_('foodSaturationLevel', 0.0),
    list_('Inventory', 10, [
        item(0, 'minecraft:bread', 2, 0),
        item(1, 'minecraft:mushroom_stew', 1, 0),
        item(2, 'minecraft:cooked_fish', 1, 1),
    ]),
)
data = bytes([10]) + name('') + root
with open('parity/fixtures/playerdata/notch_hungry.dat', 'wb') as f:
    f.write(gzip.compress(data))
