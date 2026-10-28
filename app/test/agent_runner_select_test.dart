// Per-chat agent host ("Runs on"): the default runner for new chats, the
// switch before a chat's first message, the lock after it, the host and
// model saved per chat in agent_chats.json, the model per runner, and the
// composer's runner picker. The hub is never reached: its URL is a closed
// loopback port, and the store is kept from opening an agent (no bridge
// agent needed) except where a widget test opens the screen.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/agent_chat_prefs.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/settings/controls.dart';
import 'package:notes_app/features/settings/settings_store.dart';

import 'agent_test.dart' show openAgentScreen, openAgentStore;
import 'cert_helpers.dart' show homeContext;
import 'harness.dart';
import 'sync_helpers.dart';

Finder key(String k) => find.byKey(ValueKey(k));

const _at = '2026-10-20T10:00:00Z';

/// An https hub with a device token, on a port nothing listens on.
AppSettings hubSettings() => AppSettings()
  ..clientUrl = 'https://127.0.0.1:9'
  ..clientToken = 'test-token';

/// The store of the test vault, with [settings] and [app] (the hub
/// connection). The agent is left closed: no chat is created or opened.
AgentStore storeWith(
  TestVault tv, {
  required AiSettings settings,
  required AppSettings app,
}) {
  final store = AgentStore.attach(tv.vault, settings);
  store.useAppSettings(app);
  return store;
}

/// Settings with the agent off, so no agent opens. The runner comes from the
/// vault ([chooseRunner]); a device override is no longer applied.
AiSettings closedSettings() => AiSettings()..agentEnabled = false;

/// Makes [h] the vault's runner, as Settings > AI > Run AI on does (synced).
Future<void> chooseRunner(WidgetTester t, AgentStore store, AgentHost h) =>
    real(t, () => store.setVaultDefaultHost(h));

/// A chat [id] with no messages, or with one user message.
AgentSessionState chatState(
  String id, {
  bool withMessage = false,
  bool running = false,
}) =>
    AgentSessionState(
      session: AgentSession(id: id, title: 'Chat', createdAt: _at, updatedAt: _at),
      seq: 0,
      liveSeen: 0,
      streamingText: '',
      pendingApprovals: const [],
      activity: const [],
      edits: const [],
      runs: [
        if (running)
          AgentRunRecord(
            id: 'run-$id',
            sessionId: id,
            model: 'test',
            status: 'running',
            startedAt: _at,
          ),
      ],
      messages: [
        if (withMessage)
          const AgentMessage(
            id: 'm1',
            role: 'user',
            content: 'Hello',
            text: 'Hello',
            toolCalls: [],
            isError: false,
            createdAt: _at,
          ),
      ],
    );

void main() {
  setUpAll(initCore);

  group('the saved choices of each chat', () {
    late Directory dir;
    setUp(() => dir = Directory.systemTemp.createTempSync('agent_runner_'));
    tearDown(() => dir.deleteSync(recursive: true));

    File fileNamed(String name) =>
        File('${dir.path}${Platform.pathSeparator}$name');

    test('host and model round trip through agent_chats.json', () {
      final file = fileNamed('agent_chats.json');
      final prefs = AgentChatPrefs.load(file);
      expect(prefs['a'], isNull, reason: 'no file, no choices');

      prefs.created('a', AgentHost.hub, model: 'codex/m1');
      prefs.created('b', AgentHost.local);
      prefs.pickModel('a', AgentHost.local, 'codex/m2');
      prefs.pickModel('c', AgentHost.local, 'claude-code/x');
      prefs.remove('b');

      expect(file.existsSync(), isTrue);
      expect(fileNamed('agent_chats.json.tmp').existsSync(), isFalse);

      final loaded = AgentChatPrefs.load(file);
      expect(loaded['a']?.host, AgentHost.hub, reason: 'a pick keeps the host');
      expect(loaded['a']?.model, 'codex/m2');
      expect(loaded['b'], isNull, reason: 'removed chats are forgotten');
      expect(loaded['c']?.host, AgentHost.local, reason: 'recorded on the given host');
      expect(loaded['c']?.model, 'claude-code/x');
    });

    test('a junk or unreadable agent_chats.json gives no bad choices', () {
      final file = fileNamed('agent_chats.json');
      file.writeAsStringSync(
        '{"ok":{"host":"hub","model":"m"},"bad":{"host":"moon"},'
        '"num":3,"nomodel":{"host":"local","model":5}}',
      );
      final loaded = AgentChatPrefs.load(file);
      expect(loaded['ok']?.host, AgentHost.hub);
      expect(loaded['ok']?.model, 'm');
      expect(loaded['bad'], isNull, reason: 'unknown host');
      expect(loaded['num'], isNull, reason: 'not an object');
      expect(loaded['nomodel']?.host, AgentHost.local);
      expect(loaded['nomodel']?.model, isNull, reason: 'non-string model');

      file.writeAsStringSync('not json {');
      final broken = AgentChatPrefs.load(file);
      expect(broken['ok'], isNull);
      broken.created('x', AgentHost.hub);
      expect(AgentChatPrefs.load(file)['x']?.host, AgentHost.hub,
          reason: 'a broken file is replaced on the next save');
    });

    test('a file saved by earlier versions loads and saves unchanged', () {
      final file = fileNamed('agent_chats.json');
      const saved = '{\n'
          '  "a": {\n'
          '    "host": "hub",\n'
          '    "model": "codex/m1"\n'
          '  },\n'
          '  "b": {\n'
          '    "host": "local"\n'
          '  }\n'
          '}';
      file.writeAsStringSync(saved);
      final prefs = AgentChatPrefs.load(file);
      expect(prefs['a']?.host, AgentHost.hub);
      expect(prefs['a']?.model, 'codex/m1');
      expect(prefs['b']?.host, AgentHost.local);
      expect(prefs['b']?.model, isNull);
      prefs.pickModel('a', AgentHost.local, 'codex/m1');
      expect(file.readAsStringSync(), saved,
          reason: 'same choices, same file: the format is unchanged');
    });

    // The persistence boundary: a save that fails is dropped. The choice
    // stays in memory, the old file is left as it was (the new one is
    // written beside it and renamed over it), and the next save that works
    // writes everything.
    test('a failed save keeps the choice and leaves the file intact', () {
      final file = fileNamed('agent_chats.json');
      final prefs = AgentChatPrefs.load(file);
      prefs.created('a', AgentHost.hub, model: 'codex/m1');
      final before = file.readAsStringSync();

      // A folder where the temporary file goes makes the write fail.
      final tmp = Directory('${file.path}.tmp')..createSync();
      prefs.pickModel('a', AgentHost.local, 'codex/m2');
      expect(prefs['a']?.host, AgentHost.hub);
      expect(prefs['a']?.model, 'codex/m2', reason: 'kept in memory');
      expect(file.readAsStringSync(), before, reason: 'old file untouched');
      expect(AgentChatPrefs.load(file)['a']?.model, 'codex/m1');

      tmp.deleteSync();
      prefs.created('b', AgentHost.local);
      final loaded = AgentChatPrefs.load(file);
      expect(loaded['a']?.model, 'codex/m2', reason: 'saved by the next write');
      expect(loaded['b']?.host, AgentHost.local);
    });

    test('the choices go through their storage, which may fail', () {
      final storage = _FlakyStorage({
        'a': const AgentChatPref(host: AgentHost.hub, model: 'm1'),
      });
      final prefs = AgentChatPrefs(storage: storage);
      expect(prefs['a']?.model, 'm1', reason: 'read from the storage');

      storage.fail = true;
      prefs.pickModel('a', AgentHost.local, 'm2');
      expect(prefs['a']?.model, 'm2', reason: 'a throwing write is dropped');
      expect(storage.saved['a']?.model, 'm1');

      storage.fail = false;
      prefs.remove('a');
      expect(prefs['a'], isNull);
      expect(storage.saved, isEmpty);

      final memory = AgentChatPrefs();
      memory.created('x', AgentHost.local);
      expect(memory['x']?.host, AgentHost.local, reason: 'no storage: memory only');
    });

    appTest('the store saves the model pick with the chat host', (t) async {
      await pumpApp(t, mobile: false);
      final store = AgentStore.attach(
        current.vault,
        closedSettings(),
        dataDir: dir.path,
      );
      store.useAppSettings(AppSettings());
      store.pickModel('chat-x', 'codex/store-model');
      final loaded = AgentChatPrefs.load(fileNamed('agent_chats.json'));
      expect(loaded['chat-x']?.model, 'codex/store-model');
      expect(loaded['chat-x']?.host, AgentHost.local);
      expect(store.hostOf('chat-x'), AgentHost.local);
      expect(store.modelOf('chat-x'), 'codex/store-model');
    });
  });

  appTest('a new chat runs on the default runner when it is usable', (t) async {
    final tv = await pumpApp(t, mobile: false);

    final store = storeWith(tv, settings: closedSettings(), app: hubSettings());
    await chooseRunner(t, store, AgentHost.hub);
    expect(store.usableHosts, AgentHost.values);
    expect(store.canPickHost, isTrue);
    expect(store.defaultHost, AgentHost.hub);
    expect(store.host, AgentHost.hub);

    await chooseRunner(t, store, AgentHost.local);
    expect(store.defaultHost, AgentHost.local);
    expect(store.host, AgentHost.local);
    expect(store.canPickHost, isTrue, reason: 'a usable hub can be picked');

    final unusable = storeWith(tv, settings: closedSettings(), app: AppSettings());
    await chooseRunner(t, unusable, AgentHost.hub);
    expect(unusable.usableHosts, [AgentHost.local]);
    expect(unusable.canPickHost, isFalse);
    expect(unusable.defaultHost, AgentHost.local, reason: 'no hub: this computer');
    expect(unusable.host, AgentHost.local);
  });

  appTest('switching runner before the first message moves the tab', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final store = storeWith(tv, settings: closedSettings(), app: hubSettings());
    expect(store.openId, isNull);
    expect(store.hostLocked, isFalse);
    expect(store.host, AgentHost.local);

    await real(t, () => store.pickHost(AgentHost.hub));
    expect(store.host, AgentHost.hub);
    expect(store.hostLocked, isFalse);

    await real(t, () => store.pickHost(AgentHost.local));
    expect(store.host, AgentHost.local, reason: 'back to the default');
  });

  appTest('a chat with messages keeps its runner', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final store = storeWith(tv, settings: closedSettings(), app: hubSettings());
    await chooseRunner(t, store, AgentHost.hub);
    expect(store.host, AgentHost.hub);

    store.debugSetSnapshot(chatState('chat-locked', withMessage: true));
    expect(store.hostLocked, isTrue);
    await real(t, () => store.pickHost(AgentHost.local));
    expect(store.host, AgentHost.hub, reason: 'a locked chat does not move');

    store.debugSetSnapshot(chatState('chat-empty'));
    expect(store.hostLocked, isFalse, reason: 'an empty chat can still move');
  });

  appTest('a chat still loading is locked too', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final store = storeWith(tv, settings: closedSettings(), app: hubSettings());
    store.openId = 'chat-loading';
    store.state = null;
    expect(store.hostLocked, isTrue);
    await real(t, () => store.pickHost(AgentHost.hub));
    expect(store.host, AgentHost.local);
  });

  appTest('opening a chat switches the tab to the host it was made on', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final store = storeWith(tv, settings: closedSettings(), app: hubSettings());
    await chooseRunner(t, store, AgentHost.hub);
    store.pickModel('chat-on-hub', 'codex/hub-model');
    expect(store.hostOf('chat-on-hub'), AgentHost.hub);

    await real(t, () => store.pickHost(AgentHost.local));
    expect(store.host, AgentHost.local);

    await real(t, () => store.openSession('chat-on-hub'));
    expect(store.host, AgentHost.hub);
  });

  appTest('the default model is the chosen runner\'s setting, or its first model',
      (t) async {
    final tv = await pumpApp(t, mobile: false);
    final store = storeWith(tv, settings: closedSettings(), app: AppSettings());
    store.models = const [
      AgentModel(id: 'codex/a', name: 'A'),
      AgentModel(id: 'codex/b', name: 'B'),
    ];
    await real(t, () => store.setVaultDefaultModel(AgentHost.local, 'codex/b'));
    expect(store.defaultModel, 'codex/b', reason: 'the setting is listed');
    expect(store.modelOf(null), 'codex/b');

    await real(t, () => store.setVaultDefaultModel(AgentHost.local, 'codex/gone'));
    expect(store.defaultModel, 'codex/a', reason: 'not listed: the first model');
    expect(store.modelOf(null), 'codex/a');

    store.models = const [];
    expect(store.defaultModel, 'codex/gone', reason: 'no list yet: the setting');

    await real(t, () => store.setVaultDefaultModel(AgentHost.local, null));
    expect(store.defaultModel, isNull);

    store.models = const [
      AgentModel(id: 'codex/a', name: 'A'),
      AgentModel(id: 'codex/b', name: 'B'),
    ];
    expect(store.defaultModel, 'codex/a', reason: 'no setting: the first model');

    // Custom ids are not offered any more, so a setting naming one is not listed.
    await real(
      t,
      () => store.setVaultDefaultModel(AgentHost.local, 'claude-code/custom'),
    );
    expect(store.defaultModel, 'codex/a',
        reason: 'custom ids are not listed');

    store.pickModel(null, 'codex/b');
    expect(store.modelOf(null), 'codex/b', reason: 'the next new chat');
    store.pickModel('chat-m', 'codex/a');
    expect(store.modelOf('chat-m'), 'codex/a', reason: 'the pick stays with the chat');
  });

  appTest('the runner picker is hidden with one usable runner', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final store = AgentStore.attach(tv.vault, AiSettings());
    store.useAppSettings(AppSettings());
    expect(store.canPickHost, isFalse);
    await openAgentScreen(t);
    expect(key('agent-screen'), findsOneWidget);
    expect(key('agent-runner'), findsNothing);
  });

  appTest('the runner picker shows with a usable hub and locks after a message', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final store = await openAgentStore(t);
    await openAgentScreen(t);
    // The app's sync settings get a usable hub while the local agent stays
    // the default, so the store keeps the local agent open.
    final app = tv.app.settings;
    addTearDown(() => app
      ..clientUrl = null
      ..clientToken = null);
    store.useAppSettings(app);
    app.update((s) => s
      ..clientUrl = 'https://127.0.0.1:9'
      ..clientToken = 'test-token');
    await settle(t);
    expect(store.canPickHost, isTrue);
    expect(store.host, AgentHost.local);
    expect(key('agent-runner'), findsOneWidget);
    final picker = t.widget<SelectButton<AgentHost>>(key('agent-runner'));
    expect(picker.options, AgentHost.values);
    expect(picker.value, AgentHost.local);
    expect(picker.onChanged, isNotNull, reason: 'a draft can switch');

    final s = await real(t, () => store.createSession(title: 'Runner'));
    await settle(t);
    expect(store.hostOf(s!.id), AgentHost.local, reason: 'saved with the chat');
    expect(
      t.widget<SelectButton<AgentHost>>(key('agent-runner')).onChanged,
      isNotNull,
      reason: 'an empty chat can still switch',
    );

    final run = await real(t, () => store.runScript(s.id, 'Count', '1'));
    expect(run?.status, 'completed');
    await settle(t);
    expect(store.state!.messages, isNotEmpty);
    final locked = t.widget<SelectButton<AgentHost>>(key('agent-runner'));
    expect(locked.onChanged, isNull, reason: 'a chat with messages is fixed');
    expect(locked.value, AgentHost.local, reason: 'it shows its runner');
  });

  appTest('the runner picker is disabled while a reply runs', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final store = await openAgentStore(t);
    await openAgentScreen(t);
    final app = tv.app.settings;
    addTearDown(() => app
      ..clientUrl = null
      ..clientToken = null);
    store.useAppSettings(app);
    app.update((s) => s
      ..clientUrl = 'https://127.0.0.1:9'
      ..clientToken = 'test-token');
    final s = await real(t, () => store.createSession(title: 'Busy'));
    await settle(t);
    expect(
      t.widget<SelectButton<AgentHost>>(key('agent-runner')).onChanged,
      isNotNull,
      reason: 'an empty chat can switch',
    );

    // A reply starting in the empty chat: no messages yet, but running.
    store.debugSetSnapshot(chatState(s!.id, running: true));
    await t.pump();
    expect(store.running, isTrue);
    expect(store.hostLocked, isFalse);
    expect(
      t.widget<SelectButton<AgentHost>>(key('agent-runner')).onChanged,
      isNull,
      reason: 'a running reply fixes the runner',
    );
    await real(t, () => store.pickHost(AgentHost.hub));
    expect(store.host, AgentHost.local);
  });

  appTest('opening a chat on the other host asks before stopping a local reply', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final store = storeWith(tv, settings: closedSettings(), app: hubSettings());
    // A chat made on the hub, then back to this computer.
    await real(t, () => store.pickHost(AgentHost.hub));
    store.pickModel('chat-on-hub', 'codex/hub-model');
    await real(t, () => store.pickHost(AgentHost.local));
    expect(store.host, AgentHost.local);

    store.debugSetSnapshot(chatState('chat-running', running: true));
    expect(store.running, isTrue);

    final asked = <AgentHost>[];
    var answer = false;
    Future<bool> ask(AgentHost from) async {
      asked.add(from);
      return answer;
    }

    store.addHostSwitchConfirm(ask);
    addTearDown(() => store.removeHostSwitchConfirm(ask));

    // The agent is closed, so this runs on the test clock: the snapshot's
    // viewing-scope future belongs to it and would never resolve under
    // real().
    Future<void> openOnHub() async {
      final opened = store.openSession('chat-on-hub');
      await t.pump(const Duration(milliseconds: 10));
      await opened;
    }

    await openOnHub();
    expect(asked, [AgentHost.local]);
    expect(store.host, AgentHost.local, reason: 'Cancel keeps the host');
    expect(store.openId, 'chat-running', reason: 'and the running chat');

    answer = true;
    await openOnHub();
    expect(asked, [AgentHost.local, AgentHost.local]);
    expect(store.host, AgentHost.hub, reason: 'confirmed: the tab switches');
  });

  appTest('the switch question names the running host; Cancel keeps it', (t) async {
    await pumpApp(t, mobile: false);
    final answer = confirmHostSwitch(homeContext(t), AgentHost.local);
    await settle(t);
    expect(find.text('A reply is still running on This computer.'), findsOneWidget);
    expect(find.text('Switch anyway? It will stop.'), findsOneWidget);
    await tapAndSettle(t, key('confirm-cancel'));
    expect(await answer, isFalse);
  });
}

/// Storage in memory whose writes throw while [fail] is set.
class _FlakyStorage implements AgentChatPrefsStorage {
  _FlakyStorage(Map<String, AgentChatPref> initial) : saved = Map.of(initial);

  Map<String, AgentChatPref> saved;
  bool fail = false;

  @override
  Map<String, AgentChatPref> read() => Map.of(saved);

  @override
  void write(Map<String, AgentChatPref> choices) {
    if (fail) throw const FileSystemException('write failed');
    saved = Map.of(choices);
  }
}
