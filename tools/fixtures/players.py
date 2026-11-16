# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Writes hand-made player files into parity/fixtures/playerdata/. Only the fields that
# matter are written; the server fills in the rest like for any partial player file.
#
# - notch_hungry.dat: Notch in survival mode at half hunger, holding two bread, a
#   mushroom stew and a cooked salmon.
# - notch_farmer.dat: Notch in survival mode at jeb_'s usual spawn spot, holding wheat
#   seeds, carrots, potatoes, pumpkin and melon seeds, nether wart, sugar cane and cake.
# - notch_gardener.dat: Notch in survival mode at jeb_'s usual spawn spot, holding a
#   wooden hoe, 64 bone meal, brown and red mushrooms, sunflowers
#   and cocoa beans.
# - notch_flint.dat: Notch in survival mode at jeb_'s usual spawn spot, holding flint
#   and steel (scenarios that would otherwise switch game modes from the console,
#   which races with the bot's actions).
# - notch_bed.dat: Notch in survival mode at jeb_'s usual spawn spot, holding two beds.
# - notch_potions.dat: Notch in survival mode at jeb_'s usual spawn spot, holding a
#   swiftness potion, two instant health II potions, two glass bottles and a potion with
#   a custom effect (strength II for 10 s).
# - notch_brewer.dat: Notch in survival mode at jeb_'s usual spawn spot, holding a
#   brewing stand, three water bottles and a nether wart.
# - notch_creative_flint.dat: the same in creative mode (explosions cannot hurt him).
# - notch_thorns.dat: Notch in adventure mode at jeb_'s usual spawn spot, wearing an
#   iron chestplate with Thorns XI (above level 10 thorns always strike back, for exactly
#   level - 10 damage, so no unseeded roll decides anything).
#
# Run: python tools/fixtures/players.py

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


def item(slot, item_id, count, damage, tag=None):
    fields = [byte('Count', count), short('Damage', damage), byte('Slot', slot),
              string('id', item_id)]
    if tag is not None:
        fields.append(tag)
    return compound_payload(*fields)


def compound(n, *tags):
    return tag(10, n, compound_payload(*tags))


def player(path, pos, inventory, extra=(), game_type=0):
    root = compound_payload(
        int_('playerGameType', game_type),
        int_('Dimension', 0),
        double_list('Pos', pos),
        float_list('Rotation', [0.0, 0.0]),
        byte('OnGround', 1),
        *extra,
        list_('Inventory', 10, inventory),
    )
    data = bytes([10]) + name('') + root
    with open(path, 'wb') as f:
        f.write(gzip.compress(data, mtime=0))


DIR = 'parity/fixtures/playerdata/'
player(DIR + 'notch_hungry.dat', [-473.5, 4.0, -440.5], [
    item(0, 'minecraft:bread', 2, 0),
    item(1, 'minecraft:mushroom_stew', 1, 0),
    item(2, 'minecraft:cooked_fish', 1, 1),
], (int_('foodLevel', 10), float_('foodSaturationLevel', 0.0)))
player(DIR + 'notch_flint.dat', [-471.5, 4.0, -434.5], [
    item(0, 'minecraft:flint_and_steel', 1, 0),
])
player(DIR + 'notch_bed.dat', [-471.5, 4.0, -434.5], [
    item(0, 'minecraft:bed', 2, 0),
])
player(DIR + 'notch_potions.dat', [-471.5, 4.0, -434.5], [
    item(0, 'minecraft:potion', 1, 8194),
    item(1, 'minecraft:potion', 2, 8261),
    item(2, 'minecraft:glass_bottle', 2, 0),
    item(3, 'minecraft:potion', 1, 0, compound('tag', list_('CustomPotionEffects', 10, [
        compound_payload(byte('Id', 5), byte('Amplifier', 1), int_('Duration', 200)),
    ]))),
])
player(DIR + 'notch_brewer.dat', [-471.5, 4.0, -434.5], [
    item(0, 'minecraft:brewing_stand', 1, 0),
    item(1, 'minecraft:potion', 1, 0),
    item(2, 'minecraft:potion', 1, 0),
    item(3, 'minecraft:potion', 1, 0),
    item(4, 'minecraft:nether_wart', 1, 0),
])
player(DIR + 'notch_creative_flint.dat', [-471.5, 4.0, -434.5], [
    item(0, 'minecraft:flint_and_steel', 1, 0),
], game_type=1)
player(DIR + 'notch_farmer.dat', [-471.5, 4.0, -434.5], [
    item(0, 'minecraft:wheat_seeds', 4, 0),
    item(1, 'minecraft:carrot', 2, 0),
    item(2, 'minecraft:potato', 2, 0),
    item(3, 'minecraft:pumpkin_seeds', 1, 0),
    item(4, 'minecraft:melon_seeds', 1, 0),
    item(5, 'minecraft:nether_wart', 1, 0),
    item(6, 'minecraft:reeds', 2, 0),
    item(7, 'minecraft:cake', 1, 0),
])
player(DIR + 'notch_gardener.dat', [-471.5, 4.0, -434.5], [
    item(0, 'minecraft:wooden_hoe', 1, 0),
    item(1, 'minecraft:dye', 64, 15),
    item(2, 'minecraft:brown_mushroom', 4, 0),
    item(3, 'minecraft:red_mushroom', 4, 0),
    item(4, 'minecraft:double_plant', 2, 0),
    item(5, 'minecraft:dye', 8, 3),
])
player(DIR + 'notch_thorns.dat', [-471.5, 4.0, -434.5], [
    item(102, 'minecraft:iron_chestplate', 1, 0, compound('tag', list_('ench', 10, [
        compound_payload(short('id', 7), short('lvl', 11)),
    ]))),
], game_type=2)
