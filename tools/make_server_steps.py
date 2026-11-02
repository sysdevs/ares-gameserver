"""Writes tests/servers/steps.json: the message sequence the server-link gate
plays against the login, friend and logger servers
(`servers_match`). Each step is sent on a named WebSocket connection; a step
whose message has `replyTo` waits for that reply, every step then waits for
the servers to settle. Saves come from the multi replay golden.

usage: python -I tools/make_server_steps.py
"""
import base64
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
accounts = json.loads((ROOT / 'tests/replay/multi.expected.json').read_text())['accounts']
save = {k: base64.b64encode(bytes.fromhex(v)).decode() for k, v in accounts.items()}


def b37(name):
    v = 0
    for c in name:
        v *= 37
        if 'a' <= c <= 'z':
            v += ord(c) - 96
        elif '0' <= c <= '9':
            v += ord(c) - 48 + 27
    while v % 37 == 0 and v:
        v //= 37
    return str(v)


NODE = {'w10': 10, 'w11': 11}
n = 0


def login(conn, typ, **kw):
    global n
    msg = {'type': typ, 'nodeId': NODE[conn], 'nodeTime': 1700000000000 + n * 1000, 'profile': 'main', **kw}
    if typ in ('player_login', 'player_logout'):
        n += 1
        msg['replyTo'] = f'r{n}'
    return {'conn': conn, 'msg': msg}


def plogin(conn, user, pw, reconnecting=False, has_save=False):
    return login(conn, 'player_login', nodeMembers=True, socket=f'sock-{user}-{n}', remoteAddress='10.0.0.9', uid=7,
                 username=user, password=pw, reconnecting=reconnecting, hasSave=has_save)


def friend(conn, typ, **kw):
    return {'conn': conn, 'msg': {'type': typ, **kw}}


# FriendsClientOpcodes
WORLD_CONNECT, FL_ADD, FL_DEL, IL_ADD, IL_DEL, P_LOGIN, P_LOGOUT, SETMODE, PM, PUBLIC = range(10)
R_MUTE, R_KICK, R_SHUTDOWN, R_BROADCAST, R_TRACK, R_RELOAD, R_CLEARLOGINS, R_CLEARLOGOUTS, R_QUEUESCRIPT = range(10, 19)

steps = [
    login('w10', 'world_startup'),
    plogin('w10', 'alice', 'Secret'),                 # auto-registered, no save: 4
    plogin('w10', 'bob', 'b'),
    plogin('w10', 'carol', 'c'),
    plogin('w10', 'alice', 'secret'),                 # already logged in: 3
    plogin('w11', 'alice', 'secret', reconnecting=True),  # logged in on another node: 3
    plogin('w10', 'alice', 'secret', reconnecting=True, has_save=True),  # 2, no save
    login('w10', 'player_logout', username='alice', save=save['alice']),
    plogin('w10', 'alice', 'wrong'),                  # 1
    plogin('w11', 'alice', 'secret'),                 # hop timer: 10
    plogin('w10', 'alice', 'secret'),                 # 0 with save
    login('w11', 'player_autosave', username='alice', save=save['bob']),
    login('w11', 'player_autosave', username='alice', save='AAAA'),
    login('w10', 'player_logout', username='alice', save=save['alice']),
    login('w10', 'player_mute', staff='mod', username='alice', until=1800000000000),
    login('w10', 'player_ban', staff='mod', username='nobody', until=1900000000000),
    login('w10', 'player_ban', staff='mod', username='bob', until=1900000000000),
    login('w10', 'player_force_logout', username='bob'),
    plogin('w10', 'bob', 'b'),                        # banned: 5
    plogin('w10', 'alice', 'secret'),                 # 0, muted_until set
    login('w10', 'player_logout', username='carol', save='AAAA'),
    login('w10', 'world_startup'),

    friend('f10', WORLD_CONNECT, world=10, profile='main'),
    friend('f11', WORLD_CONNECT, world=11, profile='main'),
    friend('fbad', WORLD_CONNECT, world=12, profile='other'),
    friend('f10', P_LOGIN, world=10, username37=b37('alice'), privateChat=0, staffLvl=0),
    friend('f11', P_LOGIN, world=11, username37=b37('bob'), privateChat=0, staffLvl=0),
    friend('f10', FL_ADD, world=10, username37=b37('alice'), targetUsername37=b37('bob')),
    friend('f11', FL_ADD, world=11, username37=b37('bob'), targetUsername37=b37('alice')),
    friend('f11', FL_ADD, world=11, username37=b37('bob'), targetUsername37=b37('carol')),
    friend('f10', PM, world=10, profile='main', nodeTime=1700000100000, username37=b37('alice'), targetUsername37=b37('bob'),
           staffLvl=0, pmId=1, chat='hi bob', coord=12345),
    friend('f11', SETMODE, world=11, username37=b37('bob'), privateChat=2),
    friend('f11', SETMODE, world=11, username37=b37('bob'), privateChat=7),
    friend('f10', PUBLIC, nodeId=10, profile='main', nodeTime=1700000200000, session_uuid='sess-1', coord=54321, chat='hello all'),
    friend('f10', IL_ADD, world=10, username37=b37('alice'), targetUsername37=b37('bob')),
    friend('f10', IL_ADD, world=10, username37=b37('alice'), targetUsername37=b37('dave')),
    friend('f10', IL_DEL, world=10, username37=b37('alice'), targetUsername37=b37('bob')),
    friend('f10', FL_DEL, world=10, username37=b37('alice'), targetUsername37=b37('bob')),
    friend('f11', P_LOGOUT, world=11, username37=b37('bob')),
    friend('f10', P_LOGIN, world=10, username37=b37('alice'), privateChat=1, staffLvl=2),
    friend('f11', P_LOGIN, world=11, username37=b37('carol'), privateChat=0, staffLvl=0),
    friend('relay', R_MUTE, nodeId=10, username='alice', muted_until='2027-01-01 00:00:00'),
    friend('relay', R_KICK, nodeId=11, username='bob'),
    friend('relay', R_SHUTDOWN, nodeId=10, duration=50),
    friend('relay', R_BROADCAST, nodeId=11, broadcast='hello world'),
    friend('relay', R_TRACK, nodeId=10, username='alice', state=True),
    friend('relay', R_RELOAD, nodeId=11),
    friend('relay', R_CLEARLOGINS, nodeId=10),
    friend('relay', R_CLEARLOGOUTS, nodeId=10),
    friend('relay', R_QUEUESCRIPT, nodeId=10, username='alice', scriptName='welcome'),
    friend('relay', R_BROADCAST, nodeId=99, broadcast='nobody'),
    friend('relay', 77),

    {'conn': 'log', 'msg': {'type': 'session_log', 'logs': [
        {'session_uuid': 'sess-1', 'timestamp': 1700000000000, 'coord': 1, 'event': 'login', 'event_type': 2},
        {'session_uuid': 'sess-1', 'timestamp': 1700000005000, 'coord': 2, 'event': 'walk', 'event_type': 3}]}},
    {'conn': 'log', 'msg': {'type': 'wealth_event', 'events': [
        {'session_uuid': 'sess-1', 'timestamp': 1700000010000, 'coord': 3, 'event_type': 1,
         'account_items': [{'id': 995, 'count': 10}], 'account_value': 10},
        {'session_uuid': 'sess-1', 'timestamp': 1700000011000, 'coord': 4, 'event_type': 2,
         'account_items': [], 'account_value': 0, 'recipient_session': 'sess-2',
         'recipient_items': [{'id': 1, 'count': 1}], 'recipient_value': 5}]}},
    {'conn': 'log', 'msg': {'type': 'report', 'session_uuid': 'sess-1', 'timestamp': 1700000020000, 'coord': 5, 'offender': 'bob', 'reason': 3}},
    {'conn': 'log', 'msg': {'type': 'input_track', 'session_uuid': 'sess-1', 'timestamp': 1700000030000, 'buf': base64.b64encode(bytes(range(16))).decode()}},
    {'conn': 'log', 'msg': {'type': 'mystery'}},
]

out = ROOT / 'tests/servers/steps.json'
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps(steps, indent=1) + '\n', newline='\n')
print(f'{len(steps)} steps -> {out.relative_to(ROOT)}')
