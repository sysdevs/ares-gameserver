"""Write tests/fixtures/<case>/content/ trees for the pack differential gates.

Text configs are authored here so every edge case is explicit; binary assets
(models, sprites, fonts, textures, synths, midis, title art) are copied from
the sibling LostCityRS Content checkout because the packers need real files.

Fixtures build with `build.verify = false` (see docs/testing.md): with
verify on, the title / textures / sounds / wordenc / interface / config jags
are checked against the real content CRCs, which only the full Content tree
can satisfy.

usage: python -I tools/make_pack_fixtures.py [<Content>] [<fixtures dir>]
"""
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
CONTENT = Path(sys.argv[1] if len(sys.argv) > 1 else HERE / "../../Content").resolve()
FIXTURES = Path(sys.argv[2] if len(sys.argv) > 2 else HERE / "../tests/fixtures").resolve()


def find(root: Path, name: str) -> Path:
    hits = sorted(root.rglob(name), key=lambda p: ("_unpack" in p.parts, p.as_posix()))
    if not hits:
        sys.exit(f"{name} not found under {root}")
    return hits[0]


def copy_file(src: Path, dst: Path):
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dst)


def write(dst: Path, text: str):
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(text, encoding="utf-8", newline="\n")


def pack_file(dst: Path, names: list[str]):
    write(dst, "".join(f"{i}={n}\n" for i, n in enumerate(names)))


MODELS = [
    "outdoorfurniture_fountain", "outdoorfurniture_bench",
    "woman_head_mediumhair", "chat_female_head_mediumhair",
    "man_head_bald", "chat_male_head_bald",
    "inv_boots", "man_feet_boots", "woman_feet_boots",
    "kalphite_glow",
]
ANIMS = ["anim_0", "anim_1"]
SPRITES = ["mapscene", "mapfunction", "mapdots", "tradebacking", "combatboxes"]
SYNTHS = 3


def case_minimal(root: Path):
    s = root / "scripts"
    copy_file(CONTENT / "scripts/engine.rs2", s / "engine.rs2")
    copy_file(CONTENT / "scripts/engine.constant", s / "engine.constant")
    c = s / "fixture/configs"
    write(c / "fixture.loc", """[fixture_fountain]
name=Fountain
desc=A fixture fountain.
model=outdoorfurniture_fountain
width=2
length=2
op1=Drink

[fixture_door]
name=Door
desc=A fixture door.
model=outdoorfurniture_bench
active=yes
op1=Open
""")
    write(c / "fixture.npc", """[fixture_npc]
name=Fixture
desc=A fixture npc.
walkanim=fixture_walk,fixture_walk,fixture_walk,fixture_walk
readyanim=fixture_ready
op1=Talk-to
model1=woman_head_mediumhair
head1=chat_female_head_mediumhair
""")
    write(c / "fixture.obj", """[fixture_boots]
name=Boots
desc=Fixture boots.
model=inv_boots
manwear=man_feet_boots,0
womanwear=woman_feet_boots,0
iop2=Wear
cost=500
wearpos=feet
weight=12oz

[fixture_coins]
name=Coins
model=inv_boots
stackable=yes

[fixture_quest]
name=Quest item
model=inv_boots
tradeable=no
""")
    write(c / "fixture.seq", """[fixture_ready]
frame1=anim_0

[fixture_walk]
frame1=anim_0
frame2=anim_1
""")
    write(c / "fixture.spotanim", """[fixture_glow]
model=kalphite_glow
anim=fixture_ready
""")
    write(c / "fixture.hitsplat", "[fixture_block]\nsprite=hitmarks,0\n\n[fixture_hit]\nsprite=hitmarks,1\nrecol1s=0xff0000\nrecol1d=0x00ff00\n")
    write(c / "fixture.idk", """[fixture_head]
type=man_hair
head1=chat_male_head_bald
model1=man_head_bald
""")
    write(c / "fixture.varp", "[fixture_varp]\nscope=perm\n")
    write(c / "fixture.varbit", "[fixture_bit]\nbasevar=fixture_varp\nstartbit=0\nendbit=3\n")
    write(c / "fixture.varn", "[fixture_varn]\ntype=int\n")
    write(c / "fixture.vars", "[fixture_vars]\n")
    write(c / "fixture.inv", """[fixture_shop]
scope=shared
size=5
restock=yes
stackall=yes
allstock=no
stock1=fixture_boots,2,500
""")
    write(c / "fixture.param", """[fixture_message]
type=string
default=Hello

[fixture_level]
type=int
default=1
""")
    write(c / "fixture.enum", """[fixture_enum]
inputtype=int
outputtype=namedobj
val=1,fixture_boots
""")
    write(c / "fixture.struct", "[fixture_struct]\nparam=fixture_level,5\nparam=fixture_message,Hi\n")
    write(c / "fixture.dbtable", "[fixture_table]\ncolumn=total,int\n")
    write(c / "fixture.dbrow", "[fixture_row]\ntable=fixture_table\ndata=total,5\n")
    write(c / "fixture.hunt", """[fixture_hunt]
type=player
check_vis=lineofsight
check_nottoostrong=off
check_notcombat=%fixture_varp
check_notcombat_self=%fixture_varn
check_notbusy=off
check_afk=off
find_keephunting=on
find_newmode=applayer2
""")
    write(c / "fixture.mesanim", "[fixture_chat]\nlen1=fixture_ready\nlen2=fixture_ready\nlen3=fixture_ready\nlen4=fixture_ready\n")
    write(c / "fixture.constant", "^fixture_const = 7\n")
    write(s / "floors/underlay.flo", "[grassland]\ncolour=0x35720A\n\n[muddygrass]\ncolour=0x58680B\n")
    write(s / "floors/overlay.flo", "[cliff]\noverlay=yes\ncolour=0xAAAAAA\n")
    write(s / "fixture/interfaces/fixture_if.if", "[com_0]\ntype=layer\nx=0\ny=0\nwidth=512\nheight=334\n")
    write(s / "fixture/scripts/fixture.rs2", '[opnpc1,fixture_npc]\nmes("hello <tostring(^fixture_const)>");\n')

    p = root / "pack"
    pack_file(p / "loc.pack", ["fixture_fountain", "fixture_door"])
    pack_file(p / "npc.pack", ["fixture_npc"])
    pack_file(p / "obj.pack", ["fixture_boots"])
    pack_file(p / "seq.pack", ["fixture_ready", "fixture_walk"])
    pack_file(p / "spotanim.pack", ["fixture_glow"])
    pack_file(p / "hitsplat.pack", ["fixture_block", "fixture_hit"])
    pack_file(p / "idk.pack", ["fixture_head"])
    pack_file(p / "varp.pack", ["fixture_varp"])
    pack_file(p / "varbit.pack", ["fixture_bit"])
    pack_file(p / "inv.pack", ["fixture_shop"])
    pack_file(p / "flo.pack", ["grassland", "muddygrass", "cliff"])
    pack_file(p / "interface.pack", ["fixture_if", "fixture_if:com_0"])
    write(p / "interface.order", "0\n1\n")
    pack_file(p / "model.pack", MODELS)
    pack_file(p / "anim.pack", ANIMS)
    pack_file(p / "animset.pack", ANIMS)
    pack_file(p / "base.pack", ["base_0"])
    copy_file(CONTENT / "pack/texture.pack", p / "texture.pack")

    for m in MODELS:
        copy_file(find(CONTENT / "models", f"{m}.ob2"), root / "models" / f"{m}.ob2")
    for a in ANIMS:
        copy_file(CONTENT / "models/_animset" / f"{a}.anim", root / "models/_animset" / f"{a}.anim")

    for d in ("fonts", "title", "binary", "textures"):
        shutil.copytree(CONTENT / d, root / d)
    for sp in SPRITES:
        copy_file(CONTENT / "sprites" / f"{sp}.png", root / "sprites" / f"{sp}.png")
    copy_file(CONTENT / "sprites/meta/combatboxes.opt", root / "sprites/meta/combatboxes.opt")

    synths = sorted(CONTENT.glob("synth/*.synth"))[:SYNTHS]
    for f in synths:
        copy_file(f, root / "synth" / f.name)
    pack_file(p / "synth.pack", [f.stem for f in synths])
    write(p / "synth.order", "".join(f"{i}\n" for i in range(len(synths))))

    jingle = sorted(CONTENT.glob("jingles/*.mid"))[0]
    song = sorted(CONTENT.glob("songs/*.mid"))[0]
    copy_file(jingle, root / "jingles" / jingle.name)
    copy_file(song, root / "songs" / song.name)
    pack_file(p / "midi.pack", [song.stem, jingle.stem])

    # one 64x64 mapsquare: a flat plain, one wall of the door loc, one npc, one obj
    lines = ["==== MAP ===="]
    for z in range(64):
        for x in range(64):
            tok = "h30 u0" if (x + z) % 7 else "h30 o2;0;0 u1"
            lines.append(f"0 {x} {z}: {tok}")
    lines += ["==== LOC ====", "0 10 10: 1 0 1", "0 12 12: 0", "==== NPC ====", "0 20 20: 0", "==== OBJ ====", "0 21 21: 0 3"]
    write(root / "maps/m50_50.jm2", "\n".join(lines) + "\n")
    pack_file(p / "map.pack", ["m50_50", "l50_50"])
    write(root / "maps/multiway.csv", "// none\n")
    write(root / "maps/free2play.csv", "0_50_50_0_0\n")
    write(root / "maps/ignore.csv", "")
    write(root / "maps/labels.txt", "Fixture,3239,3233,1\n")


def case_ordering(root: Path):
    """01-minimal plus names whose bytewise and NTFS (case-insensitive) orders
    differ, several configs per file, transmitted packs with gaps and missing
    names, categories across loc/npc/obj, `_unpack` and `cert_` names."""
    case_minimal(root)
    s = root / "scripts"
    # bytewise: Z_area < _area < a_area < b_area (NTFS order puts `_area` last). No two names
    # differ only by case: they would be one directory on Windows and macOS.
    for area in ["b_area", "_area", "a_area", "Z_area"]:
        c = s / area / "configs"
        tag = area.lower().strip("_")
        write(c / f"{tag}_B.loc", f"""[{tag}_loc_z]
model=outdoorfurniture_fountain
category=cat_{tag}

[{tag}_loc_a]
model=outdoorfurniture_bench
category=cat_shared
""")
        write(c / f"{tag}_a.loc", f"[{tag}_loc_m]\nmodel=outdoorfurniture_bench\n")
        write(c / f"{tag}.npc", f"[{tag}_npc]\nname=N\nmodel1=woman_head_mediumhair\ncategory=cat_npc_{tag}\n\n[{tag}_npc2]\nname=M\nmodel1=man_head_bald\ncategory=cat_shared\n")
        write(c / f"{tag}.obj", f"[{tag}_obj]\nname=O\nmodel=inv_boots\ncategory=cat_obj_{tag}\n")
        write(c / f"{tag}.enum", f"[{tag}_enum]\ninputtype=int\noutputtype=int\nval=1,2\n")
        write(c / f"{tag}.param", f"[{tag}_param]\ntype=int\ndefault=0\n")
        write(c / f"{tag}.varp", f"[{tag}_varp]\nscope=temp\n")
        sc = s / area / "scripts"
        write(sc / f"{tag}.rs2", f'[proc,{tag}_proc]\nmes("{tag}");\n\n[label,{tag}_label]\nmes("l");\n')
    # configs one level deeper (parent dir is `configs`) and an `_unpack` dir that skips the folder rule
    write(s / "deep/configs/sub/deep.loc", "[deep_loc]\nmodel=outdoorfurniture_bench\n")
    write(s / "_unpack/old/old.loc", "[unpack_loc]\nmodel=outdoorfurniture_bench\n")
    p = root / "pack"
    # transmitted packs with gaps: kept ids stay, new names append from max+1 in walk order
    write(p / "loc.pack", "0=fixture_fountain\n7=fixture_door\n3=b_area_loc_z\n")
    write(p / "npc.pack", "0=fixture_npc\n")
    # cert_ objs exist only as pack names; their configs are generated from template_for_cert
    write(s / "fixture/configs/cert.obj", "[template_for_cert]\nmodel=inv_boots\nstackable=yes\n")
    write(p / "obj.pack", "0=fixture_boots\n12=a_area_obj\n13=cert_a_area_obj\n14=template_for_cert\n20=cert_b_area_obj\n")
    write(p / "varp.pack", "0=fixture_varp\n")


def case_numbers(root: Path):
    """01-minimal plus the number/string rules of the config packers
    (docs/behaviour.md B.5-B.9): constant substitution that resumes at the old index,
    hex and >32-bit ints, fractional (oz/lb) and negative weights, sparse
    recol/seq/stock/mesanim arrays, quoted CSV, CRLF and lone-CR files,
    trailing spaces kept in values."""
    case_minimal(root)
    s = root / "scripts"
    c = s / "numbers/configs"
    # `^long_name` shortens the value, so the scan resumes past `^b` and leaves it unreplaced
    write(c / "numbers.constant", "^long_name = 1\n^b = 2\n^word = hello\n")
    write(c / "numbers.enum", """[numbers_enum_str]
inputtype=int
outputtype=string
val=^long_name,text^b
val=2,^word
val=3,a^b,^word
default=^word

[numbers_enum_auto]
inputtype=autoint
outputtype=int
val=x,0x7fffffff
val=y,-5

[numbers_enum_int]
inputtype=int
outputtype=int
val=4294967295,4294967296
val=-1,0x10
default=null
""")
    write(c / "numbers.param", "[numbers_coord]\ntype=coord\ndefault=0_50_50_10_5\n\n[numbers_big]\ntype=int\ndefault=0xFFFFFFFF\n\n[numbers_str]\ntype=string\ndefault=null\n")
    write(c / "numbers.struct", "[numbers_struct]\nparam=numbers_coord,1_49_51_63_0\nparam=numbers_big,2147483648\nparam=numbers_str,has spaces \n")
    write(c / "numbers.dbtable", '[numbers_table]\ncolumn=label,string,int\ncolumn=pair,int,int,LIST,REQUIRED,INDEXED\ncolumn=key,string,int,REQUIRED,INDEXED\ndefault=label,"a, quoted",7\n')
    write(c / "numbers.dbrow", '[numbers_row]\ntable=numbers_table\ndata=pair,1,2\ndata=pair,3,0x4\ndata=label,"x,y",-9\ndata=key,k1,7\n\n[numbers_row2]\ntable=numbers_table\ndata=pair,3,1\ndata=label,"x,y",3\ndata=key,7,1\n')
    write(c / "numbers.inv", "[numbers_inv]\nscope=temp\nsize=5\nstock1=fixture_boots,2\nstock3=fixture_boots,7,300\n")
    write(c / "numbers.mesanim", "[numbers_mesanim]\nlen1=fixture_ready\nlen1=fixture_walk\nlen2=fixture_ready\nlen4=fixture_walk\n")
    write(c / "numbers.seq", "[numbers_seq]\nframe1=anim_0\nframe3=anim_1\niframe2=anim_1\ndelay3=5\nwalkmerge=label_1,label_0,2\nreplaceheldleft=hide\n")
    write(c / "numbers.spotanim", "[numbers_spot]\nmodel=kalphite_glow\nrecol1s=0\nrecol1d=32767\nrecol2s=0x10\n")
    # CRLF file with a value that keeps its trailing space
    (c / "numbers.npc").write_bytes(
        b"[numbers_npc]\r\nname=Spaced \r\nmodel1=woman_head_mediumhair\r\nmodel3=man_head_bald\r\nhead2=chat_male_head_bald\r\n"
        b"recol1s=5\r\nrecol1d=200\r\nrecol3s=7\r\nrecol3d=8\r\nvislevel=12\r\npatrol1=0_50_50_1_2,3\r\npatrol2=1_50_50_63_63\r\n"
    )
    # lone-CR line endings: the name crawl splits on `\r?\n` only, so it sees just the first config here
    (c / "numbers.obj").write_bytes(b"[numbers_obj_kg]\rmodel=inv_boots\rweight=2kg\rrecol2s=100\rrecol2d=50\r\r[numbers_obj_hidden]\rmodel=inv_boots\r")
    # oz/lb weights are fractional grams (truncated toward zero), also negative
    write(c / "numbers_weights.obj", "[numbers_obj_lb]\nmodel=inv_boots\nweight=3lb\n\n[numbers_obj_neg]\nmodel=inv_boots\nweight=-3lb\ncost=4294967297\n\n"
          "[numbers_obj_oz]\nname=Ounce\nmodel=inv_boots\nweight=3oz\ncount1=fixture_boots,2\ncount3=fixture_boots,5\n")
    write(c / "numbers.loc", "[numbers_loc]\nmodel=outdoorfurniture_bench\nrecol2s=31\nrecol2d=0x7FFF\nforceapproach=east\nactive=no\n\n[numbers_loc_noname]\nmodel=outdoorfurniture_fountain\n")
    write(c / "numbers.varp", "[numbers_varp]\nclientcode=0x10\ntransmit=yes\nprotect=no\n")
    write(c / "numbers.hunt", "[numbers_hunt]\ntype=player\nfind_newmode=opobj2\nrate=0x2\nextracheck_var=%fixture_varp,>3\nextracheck_var=%numbers_varp,&8\n")
    write(s / "floors/flo_named.flo", "[flo_hidden]\ncolour=0x123456\n")
    p = root / "pack"
    write(p / "flo.pack", "0=grassland\n1=muddygrass\n2=cliff\n3=flo_hidden\n")


def jm2(tiles, locs=(), npcs=(), objs=()):
    """A .jm2 text: tiles {(level, x, z): token string}, locs (level, x, z, id, shape, angle), npcs (level, x, z, id), objs (level, x, z, id, count)."""
    out = ["==== MAP ===="] + [f"{l} {x} {z}: {t}" for (l, x, z), t in tiles.items()]
    if locs:
        out += ["==== LOC ===="] + [f"{l} {x} {z}: {i}" + (f" {s}" if s is not None else "") + (f" {a}" if a is not None else "") for l, x, z, i, s, a in locs]
    if npcs:
        out += ["==== NPC ===="] + [f"{l} {x} {z}: {i}" for l, x, z, i in npcs]
    if objs:
        out += ["==== OBJ ===="] + [f"{l} {x} {z}: {i} {c}" for l, x, z, i, c in objs]
    return "\n".join(out) + "\n"


def case_maps(root: Path):
    """01-minimal plus several mapsquares exercising the map and worldmap
    encoders: bridges (level-1 flag 2), overlay shapes/rotations, wall and
    decor shapes on active/inactive locs, mapscene/mapfunction, npcs hidden
    from the minimap, obj counts over 255, the underground-pass level
    exception (m33_72), an ignored map, multiway/free zones and labels.
    tests/oneoffs/*.csv feed the map csv importers (`import_csv_matches`)."""
    case_minimal(root)
    c = root / "scripts/maps/configs"
    write(c / "maps.loc", """[maps_wall]
name=Wall
model=outdoorfurniture_bench

[maps_door]
name=Door
model=outdoorfurniture_bench
active=yes
op1=Open

[maps_scenery]
model=outdoorfurniture_fountain
mapscene=8
mapfunction=3

[maps_hidden]
model=outdoorfurniture_fountain
mapscene=22
""")
    write(c / "maps.npc", "[maps_hidden_npc]\nname=Ghost\nmodel1=man_head_bald\nminimap=no\n")
    p = root / "pack"
    write(p / "loc.pack", "0=fixture_fountain\n1=fixture_door\n2=maps_wall\n3=maps_door\n4=maps_scenery\n5=maps_hidden\n")
    write(p / "npc.pack", "0=fixture_npc\n1=maps_hidden_npc\n")

    maps = root / "maps"
    for f in maps.glob("*.jm2"):
        f.unlink()
    tiles = {}
    for z in range(64):
        for x in range(64):
            tiles[(0, x, z)] = "h12 u0" if (x * z) % 5 else f"o{(x + z) % 3};{(x + z) % 12};{x % 4} u1"
    for x in range(10, 20):
        for z in range(10, 14):
            tiles[(1, x, z)] = "f2 u1"  # bridge
            tiles[(2, x, z)] = f"h{x} o2;1;{z % 4}"
    tiles[(3, 63, 63)] = "f4"
    locs = [
        (0, 5, 5, 2, 0, 1), (0, 5, 6, 3, 0, 2), (0, 6, 6, 2, 2, 3), (0, 7, 7, 3, 4, 0), (0, 8, 8, 2, 9, 1), (0, 8, 9, 3, 9, 2),
        (0, 9, 9, 4, None, None), (0, 9, 9, 0, 22, 0), (1, 12, 11, 2, 0, 0), (1, 13, 11, 4, 10, 1), (0, 20, 20, 5, None, None), (2, 15, 12, 3, 2, 1),
    ]
    npcs = [(0, 30, 30, 0), (0, 30, 30, 1), (1, 12, 12, 0), (0, 31, 2, 1)]
    objs = [(0, 40, 40, 0, 300), (0, 40, 40, 0, 1), (2, 14, 13, 0, 5)]
    write(maps / "m50_50.jm2", jm2(tiles, locs, npcs, objs))
    write(maps / "m50_51.jm2", jm2({(0, 1, 1): "h5 u1", (0, 63, 0): "o1 u0"}, [(0, 3, 3, 4, 10, 0)], [(0, 4, 4, 0)]))
    write(maps / "m33_72.jm2", jm2({(1, 0, 0): "o2;3;1", (0, 0, 0): "u0"}, [(1, 2, 2, 2, 0, 3)], objs=[(1, 5, 5, 0, 2)]))
    write(maps / "m51_50.jm2", jm2({(0, 0, 0): "u1"}))  # ignored by the worldmap
    write(p / "map.pack", "0=m50_50\n1=l50_50\n2=m50_51\n3=l50_51\n4=m33_72\n5=l33_72\n6=m51_50\n7=l51_50\n")
    write(maps / "multiway.csv", "// pvp zones\n0_50_50_8_8,0_50_50_23_15\n0_50_51_0_0\n")
    write(maps / "free2play.csv", "0_50_50_0_0,0_50_51_63_63\n0_33_72_0_0\n")
    write(maps / "ignore.csv", "0_51_50_0_0\n")
    write(maps / "labels.txt", "=Fixture Town,3215,3218,1\n=North,3205,3290,0\nnot a label\n=Short,3200,0,0\n")

    o = FIXTURES.parent / "oneoffs"
    write(o / "npc.csv", "x,z,level,id\n3210,3210,0,0\n3205,3204,1,1\n// comment\n3201,3263,0,0\n9999,9999,0,0\n2116,4609,1,0\n")
    write(o / "obj.csv", "x,z,level,id,quantity\n3210,3210,0,0,5\n3220,3200,0,0,2\n3201,3263,0,0,10\n9999,9999,0,0,1\n")


VM_RS2 = """[proc,vm_arith](int $a, int $b)(int, int, int, int, int, int, int, int, int, int, int, int, int, int)
return(add($a, $b), sub($a, $b), multiply($a, $b), divide($a, $b), modulo($a, $b), pow($a, $b), invpow($a, $b), and($a, $b), or($a, $b), min($a, $b), max($a, $b), scale($a, $b, 7), addpercent($a, $b), interpolate($a, $b, 1, 5, 3));

[proc,vm_bits](int $v, int $bit, int $start, int $end)(int, int, int, int, int, int, int, int, int, int)
return(setbit($v, $bit), clearbit($v, $bit), testbit($v, $bit), togglebit($v, $bit), setbit_range($v, $start, $end), clearbit_range($v, $start, $end), getbit_range($v, $start, $end), setbit_range_toint($v, $bit, $start, $end), bitcount($v), abs($v));

[proc,vm_trig](int $x, int $y)(int, int, int)
return(sin_deg($x), cos_deg($x), atan2_deg($x, $y));

[proc,vm_random](int $n)(int, int, int)
return(random($n), randominc($n), random($n));

[proc,vm_strings](string $a, string $b, int $n)(string, string, string, string, string, int, int, string, int, int, string)
return(append($a, $b), append_num($a, $n), append_signnum($a, $n), lowercase($a), tostring($n), compare($a, $b), string_length($a), substring($a, $n, add($n, 3)), string_indexof_char($a, 'l'), string_indexof_string($b, $a), append_char($a, 'z'));

[proc,vm_enum](int $key)(string, int, int)
return(enum(int, string, numbers_enum_str, $key), enum(int, int, numbers_enum_int, $key), enum_getoutputcount(numbers_enum_str));

[proc,vm_struct]()(coord, int, string)
return(struct_param(numbers_struct, numbers_coord), struct_param(numbers_struct, numbers_big), struct_param(numbers_struct, numbers_str));

[proc,vm_db_find](int $k)(int, dbrow, dbrow, dbrow)
def_int $count = db_find_with_count(numbers_table:pair:0, $k);
return($count, db_findnext, db_findnext, db_findnext);

[proc,vm_db_key](string $s)(int, dbrow, int, dbrow)
def_int $count = db_find_with_count(numbers_table:key:0, $s);
def_dbrow $row = db_findnext;
return($count, $row, db_find_refine_with_count(numbers_table:key:1, 7), db_findbyindex(0));

[proc,vm_db_list]()(int, dbrow, dbrow, dbrow)
def_int $count = db_listall_with_count(numbers_table);
return($count, db_findnext, db_findnext, db_findnext);

[proc,vm_db_field](dbrow $row, int $i)(string, int, int, int, int, dbtable)
def_string $label;
def_int $num;
$label, $num = db_getfield($row, numbers_table:label, $i);
def_int $a;
def_int $b;
$a, $b = db_getfield($row, numbers_table:pair, $i);
return($label, $num, db_getfieldcount($row, numbers_table:pair), $a, $b, db_getrowtable($row));

[proc,vm_split](string $text, int $width, int $lines)(int, int, string, string, seq)
split_init($text, $width, $lines, p12_full);
return(split_pagecount, split_linecount(0), split_get(0, 0), split_get(0, 1), split_getanim(0));

[proc,vm_oc](obj $o)(string, string, string, boolean, int, int, int, int, int, boolean, obj, obj, boolean)
return(oc_name($o), oc_desc($o), oc_debugname($o), oc_members($o), oc_weight($o), oc_wearpos($o), oc_wearpos2($o), oc_wearpos3($o), oc_cost($o), oc_tradeable($o), oc_cert($o), oc_uncert($o), oc_stackable($o));

[proc,vm_nc](npc $n)(string, string, string, category, int, string)
return(nc_name($n), nc_desc($n), nc_debugname($n), nc_category($n), nc_size($n), nc_op($n, 1));

[proc,vm_lc](loc $l)(string, string, string, category, int, int)
return(lc_name($l), lc_desc($l), lc_debugname($l), lc_category($l), lc_width($l), lc_length($l));

[proc,vm_flow](int $n)(int)
def_int $acc = 0;
def_int $i = 0;
while ($i < $n) {
    $acc = add($acc, ~vm_sq($i));
    $i = add($i, 1);
}
switch_int ($n) {
    case 1 : $acc = add($acc, 100);
    case 2, 3 : $acc = add($acc, 200);
    case default : $acc = add($acc, 5);
}
return($acc);

[proc,vm_sq](int $x)(int)
return(multiply($x, $x));

[proc,vm_recurse](int $n)(int)
return(~vm_recurse(add($n, 1)));

[proc,vm_spin]()(int)
def_int $i = 0;
while (1 = 1) {
    $i = add($i, 1);
}
return($i);

[proc,vm_error](string $s)
error($s);

[proc,vm_substr](string $s, int $a, int $b)(string, int, int)
return(substring($s, $a, $b), string_indexof_string($s, substring($s, $b, $a)), compare(substring($s, $a, $b), $s));
"""


def case_vm(root: Path):
    """03-numbers plus procs that exercise the script VM's pure op families
    (core control flow, number, string, enum/struct, db, config lookups,
    errors); `vm_corpus_matches` runs them through the VM."""
    case_numbers(root)
    write(root / "scripts/vm/scripts/vm.rs2", VM_RS2)

NPCS_NPC = """[npcs_hunter]
name=Hunter
model1=man_head_bald
op1=Attack
huntmode=npcs_hunt_prey
huntrange=6
defaultmode=none
maxrange=8
hitpoints=10

[npcs_prey]
name=Prey
model1=woman_head_mediumhair
category=npcs_prey_cat
op2=Pet
wanderrange=3
hitpoints=20
attack=5

[npcs_spawner]
name=Spawner
model1=man_head_bald
moverestrict=nomove
defaultmode=none
timer=5

[npcs_temp]
name=Temp
model1=man_head_bald
op2=Poke
wanderrange=1
attack=3

[npcs_looter]
name=Looter
model1=man_head_bald
huntmode=npcs_hunt_obj
huntrange=5
maxrange=6

[npcs_locker]
name=Locker
model1=man_head_bald
huntmode=npcs_hunt_loc
huntrange=4
defaultmode=none
moverestrict=nomove

[npcs_patroller]
name=Patroller
model1=man_head_bald
defaultmode=patrol
patrol1=0_50_50_30_30,2
patrol2=0_50_50_34_30,0
patrol3=0_50_50_34_36,3

[npcs_big]
name=Big
model1=man_head_bald
size=2
blockwalk=all
wanderrange=4

[npcs_ghost]
name=Ghost
model1=man_head_bald
moverestrict=passthru
blockwalk=none
wanderrange=5

[npcs_rock]
name=Rock
model1=man_head_bald
moverestrict=blocked
wanderrange=2

[npcs_indoor]
name=Indoor
model1=man_head_bald
moverestrict=indoors
wanderrange=2

[npcs_changer]
name=Changer
model1=man_head_bald
timer=3
respawnrate=8
wanderrange=3
hitpoints=15

[npcs_follower]
name=Follower
model1=man_head_bald
huntmode=npcs_hunt_follow
huntrange=7
attackrange=2
maxrange=9
givechase=no

[npcs_tester]
name=Tester
model1=man_head_bald
op1=Talk-to
moverestrict=nomove
defaultmode=none
timer=2
hitpoints=30

[npcs_stuck]
name=Stuck
model1=man_head_bald
defaultmode=patrol
patrol1=0_50_50_44_24,0
patrol2=0_50_50_40_26,1
patrol3=1_50_50_40_30,0

[npcs_statue]
name=Statue
model1=man_head_bald
op2=Look
moverestrict=nomove
defaultmode=none
"""

NPCS_HUNT = """[npcs_hunt_prey]
type=npc
check_npc=npcs_prey
check_vis=lineofsight
nobodynear=keephunting
find_newmode=opnpc1
find_keephunting=off

[npcs_hunt_follow]
type=npc
check_category=npcs_prey_cat
check_vis=lineofwalk
nobodynear=keephunting
find_newmode=apnpc1
rate=3

[npcs_hunt_obj]
type=obj
check_obj=fixture_boots
nobodynear=keephunting
find_newmode=opobj1

[npcs_hunt_loc]
type=scenery
check_loc=fixture_fountain
nobodynear=keephunting
find_newmode=queue3
find_keephunting=on
rate=4
"""

NPCS_RS2 = """[ai_opnpc1,npcs_hunter]
npc_say("Gotcha <tostring(.npc_stat(hitpoints))>");
npc_anim(fixture_ready, 0);
.npc_say("Eek");
.npc_damage(fixture_hit, 2);
npc_setmode(none);
npc_sethuntmode(npcs_hunt_prey);

[ai_apnpc1,npcs_follower]
npc_say("Near <tostring(npc_range(.npc_coord))>");
npc_facesquare(.npc_coord);
npc_setmode(wander);

[ai_opobj1,npcs_looter]
npc_say("Shiny");
npc_setmode(none);
npc_sethuntmode(npcs_hunt_obj);

[ai_queue3,npcs_locker]
%fixture_varn = add(%fixture_varn, 1);
def_int $p = modulo(%fixture_varn, 3);
if ($p = 0) {
    if (loc_find(0_50_50_12_12, fixture_fountain) = true) {
        loc_anim(fixture_ready);
        loc_change(fixture_door, 3);
    }
} else if ($p = 1) {
    loc_add(0_50_50_14_12, fixture_fountain, 1, centrepiece_straight, 5);
    npc_say("<loc_name> <tostring(loc_angle)>");
} else if (loc_find(0_50_50_14_12, fixture_fountain) = true) {
    loc_del(2);
}
loc_findallzone(0_50_50_12_12);
while (loc_findnext = true) {
    npc_say("loc <tostring(loc_angle)> <tostring(coordx(loc_coord))>");
}

[ai_timer,npcs_spawner]
def_int $phase = modulo(map_clock, 20);
if ($phase < 5) {
    npc_queue(2, $phase, 2);
    npc_add(movecoord(npc_coord, 2, 0, 1), npcs_temp, 6);
} else if ($phase < 10) {
    npc_findallany(npc_coord, 10, 0);
    while (.npc_findnext = true) {
        .npc_say("<.npc_name> <tostring(.npc_stat(hitpoints))>");
        .npc_statsub(hitpoints, 1, 10);
    }
}

[ai_queue5,npcs_tester]
npc_say("q5");

[ai_queue2,npcs_spawner]
npc_say("queued");

[ai_spawn,npcs_temp]
npc_say("Hi");
npc_walktrigger(4, 9);
npc_walk(movecoord(npc_coord, 2, 0, 0));

[ai_queue4,npcs_temp]
npc_say("walked");
npc_delay(2);
npc_say("woke");

[ai_despawn,npcs_temp]
spotanim_map(fixture_glow, npc_coord, 0, 0);

[ai_timer,npcs_changer]
def_int $p = modulo(map_clock, 30);
if ($p < 3) {
    npc_changetype(npcs_prey, 4);
} else if ($p < 6) {
    npc_walk(map_findsquare(npc_coord, 1, 3, 2));
} else if ($p < 9) {
    npc_changetype_keepall(npcs_rock, 3);
} else if ($p < 12) {
    npc_del;
} else if ($p < 18) {
    npc_walk(map_findsquare(npc_coord, 0, 2, 0));
    npc_arrivedelay;
    npc_say("arrived");
} else {
    npc_walk(map_findsquare(npc_coord, 2, 4, 1));
}

[ai_timer,npcs_tester]
%fixture_varn = add(%fixture_varn, 1);
def_int $p = modulo(%fixture_varn, 8);
if ($p = 0) {
    npc_queue(5, 7, 3);
    npc_findallany(npc_coord, 12, 0);
    npc_delay(0);
    if (.npc_findnext = true) {
        npc_say("stale");
    }
} else if ($p = 1) {
    world_delay(2);
    npc_say("world <tostring(map_clock)>");
} else if ($p = 2) {
    if (npc_hasop(2) = false) {
        npc_say("noop");
    }
} else if ($p = 3) {
    if (.npc_find(npc_coord, npcs_prey, 15, 1) = true) {
        .npc_tele(movecoord(.npc_coord, 1, 0, 1));
    }
    if (.npc_hunt(npc_coord, 15, 0) = true) {
        npc_say(.npc_name);
    }
    if (.npc_findcat(npc_coord, npcs_prey_cat, 15, 2) = true) {
        .npc_queue(2, 5, 1);
    }
    if (.npc_find(npc_coord, npcs_statue, 15, 0) = true) {
        .npc_say("statue");
    }
} else if ($p = 4) {
    npc_huntall(npc_coord, 15, 0);
    while (.npc_findnext = true) {
        .npc_anim(fixture_walk, 1);
        projanim_npc(npc_coord, .npc_uid, fixture_glow, 10, 20, 0, 30, 16, 64);
    }
    npc_findallzone(npc_coord);
    while (.npc_findnext = true) {
        .npc_statheal(hitpoints, 1, 0);
    }
    if (.npc_findexact(0_50_50_20_20, fixture_npc) = true) {
        .npc_say("exact");
    }
} else if ($p = 5) {
    npc_findall(npc_coord, npcs_temp, 15, 0);
    while (.npc_findnext = true) {
        .npc_statadd(attack, 2, 50);
        .npc_say("<tostring(.npc_stat(attack))> <tostring(.npc_basestat(attack))>");
    }
    spotanim_map(fixture_glow, movecoord(npc_coord, 1, 0, 1), 10, 5);
    projanim_map(npc_coord, movecoord(npc_coord, 3, 0, 3), fixture_glow, 10, 20, 0, 30, 16, 64);
    spotanim_npc(fixture_glow, 5, 1);
} else if ($p = 6) {
    npc_say("<tostring(npc_param(fixture_level))> <tostring(npc_attackrange)> <tostring(distance(npc_coord, 0_50_50_20_20))> <tostring(seqlength(fixture_walk))> <tostring(coordz(npc_coord))>");
    if (map_blocked(npc_coord) = false & lineofsight(npc_coord, 0_50_50_20_20) = true) {
        npc_say("clear");
    }
    if (inzone(0_50_50_0_0, 0_50_50_63_63, npc_coord) = true & lineofwalk(npc_coord, 0_50_50_35_33) = true) {
        npc_say("inside");
    }
    npc_setmode(opplayer1);
    npc_settimer(3);
} else {
    npc_setmode(applayer2);
    npc_settimer(2);
    npc_damage(fixture_hit, 3);
    npc_say("<tostring(npc_stat(hitpoints))> <tostring(npc_findhero)>");
}
"""


PLAYER_RS2 = """[login,_]
mes("Welcome to the fixture.");
last_login_info;
if_settab(fixture_if, 0);
if_settext(fixture_if:com_0, "Hello\\\\n@red@World\\\\nNext line");
if_setcolour(fixture_if:com_0, 0xff8000);
inv_add(inv, fixture_boots, 1);
inv_add(inv, fixture_coins, 1000);
inv_add(player_bank, fixture_coins, 5);
inv_transmit(inv, fixture_if:com_0);
inv_add(worn, fixture_boots, 1);
buildappearance(worn);
setidkit(fixture_head, 3);
stat_advance(attack, 5000);
stat_boost(strength, 2, 10);
stat_boost(strength, 2, 10);
%player_perm_a = 5;
%player_perm_c = 300;
%fixture_bit = 6;
settimer(fixture_timer, 5);
queue(fixture_queue, 3, 7);
weakqueue(fixture_weak, 1, 2);
obj_add(movecoord(coord, 1, 0, 0), fixture_coins, 50, 100);
obj_add(movecoord(coord, 0, 0, 1), fixture_boots, 2, 100);
say("hi");
anim(fixture_ready, 0);
spotanim_pl(fixture_glow, 50, 0);
mes("<tostring(inv_total(inv, fixture_coins))> <tostring(inv_freespace(inv))> <tostring(stat(attack))> <tostring(stat_base(strength))> <tostring(%fixture_bit)>");

[timer,fixture_timer]
mes("tick <tostring(map_clock)> <tostring(gettimer(fixture_timer))>");
stat_sub(attack, 1, 0);
if (stat(attack) < 3) {
    cleartimer(fixture_timer);
}

[queue,fixture_queue](int $arg)
mes("queued <tostring($arg)> <tostring(getqueue(fixture_queue))>");
inv_del(inv, fixture_coins, 100);
p_delay(2);
mes("after delay <tostring(inv_total(inv, fixture_coins))>");
if_settab(null, 0);

[queue,fixture_weak](int $arg)
mes("weak <tostring($arg)>");

[logout,_]
mes("Goodbye.");
"""


def case_npcs(root: Path):
    """01-minimal plus NPCs that exercise the NPC AI with no players: hunts
    of NPCs/objs/locs, op/ap interactions between NPCs, wander/patrol modes,
    move restrictions and sizes, temporary NPCs with spawn/despawn scripts,
    type changes, deletes and respawns, and the NPC/loc/server ops
    (iterators, delays, errors); `npcsim_matches` runs the World."""
    case_minimal(root)
    c = root / "scripts/npcs/configs"
    write(c / "npcs.npc", NPCS_NPC)
    write(c / "npcs.hunt", NPCS_HUNT)
    write(c / "player.inv", "[inv]\nscope=perm\nsize=28\nrunweight=yes\n\n[worn]\nscope=perm\nsize=14\n\n[player_bank]\nscope=perm\nsize=200\nstackall=yes\n\n[player_temp]\nscope=temp\nsize=8\n")
    write(c / "player.varp", "[player_perm_a]\nscope=perm\n\n[player_temp_b]\n\n[player_perm_c]\nscope=perm\ntransmit=yes\n\n[player_perm_coord]\nscope=perm\ntype=coord\n")
    write(root / "scripts/npcs/scripts/npcs.rs2", NPCS_RS2)
    write(root / "scripts/npcs/scripts/player.rs2", PLAYER_RS2)
    names = ["fixture_npc", "npcs_hunter", "npcs_prey", "npcs_spawner", "npcs_temp", "npcs_looter", "npcs_locker", "npcs_patroller", "npcs_big", "npcs_ghost", "npcs_rock", "npcs_indoor", "npcs_changer", "npcs_follower", "npcs_tester", "npcs_statue", "npcs_stuck"]
    pack_file(root / "pack/npc.pack", names)
    tiles = {}
    for z in range(64):
        for x in range(64):
            tiles[(0, x, z)] = "h30 u0" if (x + z) % 7 else "h30 o2;0;0 u1"
    locs = [(0, 10, 10, 1, 0, 1), (0, 12, 12, 0, None, None), (0, 26, 25, 1, 0, 0), (0, 27, 25, 1, 0, 0), (0, 44, 24, 0, None, None), (0, 14, 12, 1, 0, 0)]
    ids = {n: i for i, n in enumerate(names)}
    spawns = [(20, 20, "fixture_npc"), (24, 24, "npcs_hunter"), (27, 24, "npcs_prey"), (28, 27, "npcs_prey"), (22, 30, "npcs_prey"), (40, 40, "npcs_spawner"), (18, 21, "npcs_looter"),
              (12, 15, "npcs_locker"), (30, 30, "npcs_patroller"), (45, 20, "npcs_big"), (46, 23, "npcs_ghost"), (50, 50, "npcs_rock"), (52, 52, "npcs_indoor"), (36, 44, "npcs_changer"),
              (25, 28, "npcs_follower"), (33, 33, "npcs_tester"), (31, 33, "npcs_statue"), (35, 33, "npcs_statue"), (33, 32, "npcs_ghost"), (41, 24, "npcs_stuck")]
    npcs = [(0, x, z, ids[n]) for x, z, n in spawns]
    objs = [(0, 21, 21, 0, 3), (0, 16, 22, 0, 1)]
    write(root / "maps/m50_50.jm2", jm2(tiles, locs, npcs, objs))


VARO_RS2 = """[proc,varo_default_reads]()(int, player_uid)
if (inv_slotobj(inv, 0) = true) {
    return(%test_charges, %test_owner);
}
return(-2, uid);

[proc,varo_write_read]()(int, int)
def_int $a = -2;
def_int $b = -2;
if (inv_slotobj(inv, 0) = true) {
    %test_charges = 5;
    $a = %test_charges;
}
if (inv_slotobj(inv, 1) = true) {
    $b = %test_charges;
}
return($a, $b);

[proc,varo_slotobj_empty]()(boolean)
return(inv_slotobj(inv, 3));

[proc,varo_pointer_follows_slot]()(int)
if (inv_slotobj(inv, 0) = true) {
    inv_movetoslot(inv, inv, 0, 5);
    return(%test_charges);
}
return(-2);

[proc,varo_dot_form]()(int)
if (.inv_slotobj(inv, 0) = true) {
    return(.%test_charges);
}
return(-2);

[opheld1,test_ring]
%fixture_varp = %test_charges;

[proc,varo_charge](inv $inv, int $slot)
if (inv_slotobj($inv, $slot) = true) {
    %test_charges = 7;
}

[proc,varo_read](inv $inv, int $slot)(int)
if (inv_slotobj($inv, $slot) = true) {
    return(%test_charges);
}
return(-2);

[proc,varo_movetoslot_swap]()(int, int)
~varo_charge(inv, 0);
inv_movetoslot(inv, inv, 0, 5);
return(~varo_read(inv, 5), ~varo_read(inv, 0));

[proc,varo_movefromslot_to_worn]()(int)
~varo_charge(inv, 0);
inv_movefromslot(inv, worn, 0);
return(~varo_read(worn, 0));

[proc,varo_bank_deposit_no_merge]()(int, int, int, int)
~varo_charge(inv, 0);
inv_movefromslot(inv, player_bank, 0);
inv_movefromslot(inv, player_bank, 1);
inv_add(player_bank, test_coins, 3);
return(~varo_read(player_bank, 0), ~varo_read(player_bank, 1), inv_getnum(player_bank, 0), inv_getnum(player_bank, 1));

[proc,varo_both_moveinv]()(int)
~varo_charge(inv, 1);
both_moveinv(inv, worn);
if (.inv_slotobj(worn, 1) = true) {
    return(.%test_charges);
}
return(-2);

[proc,varo_moveitem_moves_instance]()(int, int, int)
~varo_charge(inv, 1);
inv_moveitem(inv, player_bank, test_ring, 2);
return(~varo_read(player_bank, 0), ~varo_read(player_bank, 1), inv_total(player_bank, test_ring));

[proc,varo_moveitem_partial_stack_copies]()(int, int, int, int)
~varo_charge(inv, 0);
inv_moveitem(inv, worn, test_coins, 4);
return(~varo_read(inv, 0), inv_getnum(inv, 0), ~varo_read(worn, 0), inv_getnum(worn, 0));

[proc,varo_moveitem_cert_drops]()(int)
~varo_charge(inv, 0);
inv_moveitem_cert(inv, player_bank, test_ring, 1);
return(~varo_read(player_bank, 0));

[proc,varo_changeslot_keeps]()(int)
~varo_charge(inv, 0);
inv_changeslot(inv, test_ring, test_coins, 1);
return(~varo_read(inv, 0));

[proc,varo_drop_discards]()(int)
~varo_charge(inv, 0);
inv_dropslot(inv, coord, 0, 100);
inv_add(inv, test_ring, 1);
return(~varo_read(inv, 0));

[proc,varo_stock_zero_clears_vars]()(int)
~varo_charge(varo_stock, 0);
inv_del(varo_stock, test_coins, 3);
inv_add(varo_stock, test_coins, 3);
return(~varo_read(varo_stock, 0));

[proc,varo_bank_uncert_roundtrip]()(int)
~varo_charge(inv, 0);
inv_moveitem_uncert(inv, player_bank, test_ring, 1);
inv_moveitem_uncert(player_bank, inv, test_ring, 1);
return(~varo_read(inv, 0));

[proc,varo_shared_inv_drops_vars]()(int, int, int)
~varo_charge(inv, 0);
~varo_charge(inv, 1);
inv_movefromslot(inv, varo_shared, 0);
inv_moveitem(inv, varo_shared, test_ring, 1);
return(~varo_read(varo_shared, 0), inv_getnum(varo_shared, 0), inv_total(varo_shared, test_ring));
"""


def case_varo(root: Path):
    """01-minimal plus varo configs, a ring and a stackable obj, the player
    invs (one shared); `tests/varo.rs` runs the procs."""
    case_minimal(root)
    s = root / "scripts"
    c = s / "varo/configs"
    write(c / "player.inv", "[inv]\nscope=perm\nsize=28\nrunweight=yes\n\n[worn]\nscope=perm\nsize=14\n\n[player_bank]\nscope=perm\nsize=200\nstackall=yes\n\n[varo_stock]\nscope=temp\nsize=5\nstock1=test_coins,3\n\n[varo_shared]\nscope=shared\nsize=5\nstackall=yes\n")
    write(c / "varo.varo", "[test_charges]\ntype=int\n\n[test_owner]\ntype=player_uid\n")
    write(c / "varo.obj", "[test_ring]\nname=Ring\nmodel=inv_boots\niop1=Rub\n\n[test_coins]\nname=Test coins\nmodel=inv_boots\nstackable=yes\n")
    write(s / "varo/scripts/varo.rs2", VARO_RS2)
    # cert_ objs exist only as pack names; ids 0-4 keep what the walk assigned before
    write(c / "cert.obj", "[template_for_cert]\nmodel=inv_boots\nstackable=yes\n")
    pack_file(root / "pack/obj.pack", ["fixture_boots", "fixture_coins", "fixture_quest", "test_ring", "test_coins", "template_for_cert", "cert_test_ring"])


CASES = {"01-minimal": case_minimal, "02-ordering": case_ordering, "03-numbers": case_numbers, "04-maps": case_maps, "05-vm": case_vm, "06-npcs": case_npcs, "07-varo": case_varo}

if __name__ == "__main__":
    for name, build in CASES.items():
        root = FIXTURES / name / "content"
        shutil.rmtree(FIXTURES / name, ignore_errors=True)
        build(root)
        print(name, sum(1 for p in root.rglob("*") if p.is_file()), "files")
