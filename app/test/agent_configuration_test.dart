import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/settings/controls.dart';
import 'package:notes_app/shell/home.dart';

import 'agent_test.dart'
    show fakeAgentSettings, openAgentStore, openAgentScreen;
import 'ai_test.dart' show aiStore, closeAi;
import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);

  test(
    'legacy model settings default to all shown and extra models persist',
    () {
      final dir = Directory.systemTemp.createTempSync('notes-agent-models-');
      addTearDown(() => dir.deleteSync(recursive: true));
      final file = File('${dir.path}/ai_settings.json')
        ..writeAsStringSync('{"agentModel":"codex/model"}');
      final settings = AiSettings.load(file);
      expect(settings.agentHiddenModels, isEmpty);
      expect(settings.agentCustomModels, isEmpty);
      settings.update((s) {
        s.agentHiddenModels.add('codex/model');
        s.agentCustomModels.add('claude-code/custom');
      });
      final restored = AiSettings.load(file);
      expect(restored.agentHiddenModels, {'codex/model'});
      expect(restored.agentCustomModels, ['claude-code/custom']);
      expect(restored.agentModel, 'codex/model');
    },
  );

  appTest(
    'picker hides models, retains the current model and adds custom ids',
    (t) async {
      await pumpApp(t, mobile: false);
      final store = await openAgentStore(t);
      store.models = const [
        AgentModel(id: 'codex/shown', name: 'Shown'),
        AgentModel(id: 'codex/hidden', name: 'Hidden'),
        AgentModel(id: 'codex/current', name: 'Current'),
      ];
      store.settings.update((s) {
        s.agentHiddenModels = {'codex/hidden', 'codex/current'};
        s.agentCustomModels = ['claude-code/custom'];
      });
      store.pickModel(null, 'codex/current');
      await openAgentScreen(t);
      final picker = t.widget<SelectButton<String>>(key('agent-model'));
      expect(picker.options, [
        'codex/shown',
        'codex/current',
        'claude-code/custom',
      ]);
      expect(picker.value, 'codex/current');
      expect(store.modelLabel('claude-code/opus'), 'Opus (latest)');
      expect(store.modelLabel('claude-code/claude-fable-5-1'), 'Fable 5.1');
      expect(store.modelLabel('codex/shown'), 'Shown');
    },
  );

  appTest('hiding the implicit current model keeps it selected and shown', (
    t,
  ) async {
    await pumpApp(t, mobile: false);
    final store = await openAgentStore(
      t,
      settings: fakeAgentSettings()..agentModel = '',
    );
    store.models = const [
      AgentModel(id: 'codex/implicit', name: 'Implicit'),
      AgentModel(id: 'codex/other', name: 'Other'),
    ];
    store.settings.update((s) => s.agentHiddenModels.add('codex/implicit'));
    expect(store.modelOf(null), 'codex/implicit');
    await openAgentScreen(t);
    final picker = t.widget<SelectButton<String>>(key('agent-model'));
    expect(picker.value, 'codex/implicit');
    expect(picker.options, ['codex/implicit', 'codex/other']);
  });

  appTest('Models switches and custom Add/Remove save on this device', (
    t,
  ) async {
    final tv = await pumpApp(t, mobile: false);
    final file = File('${tv.dir.path}/model-settings.json');
    final settings = AiSettings.load(file);
    final fake = fakeAgentSettings();
    settings
      ..agentEnabled = fake.agentEnabled
      ..agentModel = fake.agentModel
      ..agentBunPath = fake.agentBunPath
      ..agentClaudeCodeDir = fake.agentClaudeCodeDir;
    final store = await openAgentStore(t, settings: settings);
    final ai = await aiStore(t, settings: settings);
    try {
      await tapAndSettle(t, key('sidebar-settings'));
      await tapAndSettle(t, key('settings-nav-ai'));
      final visible = key('agent-model-visible-claude-code/claude-opus-5-5');
      await t.ensureVisible(visible);
      await tapAndSettle(t, visible);
      expect(
        AiSettings.load(file).agentHiddenModels,
        contains('claude-code/claude-opus-5-5'),
      );
      await t.ensureVisible(key('agent-model-add-field'));
      await enterField(
        t,
        'agent-model-add-field',
        'claude-code/claude-opus-4-7',
      );
      await tapAndSettle(t, key('agent-model-add'));
      expect(AiSettings.load(file).agentCustomModels, [
        'claude-code/claude-opus-4-7',
      ]);
      expect(store.shownModels(), contains('claude-code/claude-opus-4-7'));
      final defaults = t.widget<SelectButton<String>>(
        key('agent-default-model'),
      );
      expect(defaults.options, isNot(contains('claude-code/claude-opus-5-5')));
      expect(defaults.options, contains('claude-code/claude-opus-4-7'));
      final remove = key('agent-model-remove-claude-code/claude-opus-4-7');
      await t.ensureVisible(remove);
      await tapAndSettle(t, remove);
      expect(AiSettings.load(file).agentCustomModels, isEmpty);
    } finally {
      await closeAi(t, ai);
    }
  });

  appTest('a missing Claude login is a readable failed run', (t) async {
    await pumpApp(t, mobile: false);
    final store = await openAgentStore(t);
    final session = (await real(t, () => store.createSession(title: 'Login')))!;
    await openAgentScreen(t);
    store.debugSetSnapshot(
      AgentSessionState(
        session: session,
        messages: const [],
        runs: [
          AgentRunRecord(
            id: 'login-error',
            sessionId: session.id,
            model: 'claude-code/opus',
            status: 'failed',
            startedAt: '2026-10-09T12:00:00Z',
            endedAt: '2026-10-09T12:00:01Z',
            error: 'Claude Code: please run /login',
          ),
        ],
        pendingApprovals: const [],
        activity: const [],
        edits: const [],
        seq: 0,
        liveSeen: 0,
        streamingText: '',
      ),
    );
    await settle(t);
    expect(find.textContaining('please run /login'), findsOneWidget);
    expect(store.running, isFalse);
    expect(key('agent-stop'), findsNothing);
  });

  for (final panel in [false, true]) {
    appTest(
      'chat mention sends context and its bubble opens chat (${panel ? 'panel' : 'screen'})',
      (t) async {
        await pumpApp(t, mobile: false);
        final store = await openAgentStore(t);
        final referenced = (await real(
          t,
          () => store.createSession(title: 'Garden chat'),
        ))!;
        final currentChat = (await real(
          t,
          () => store.createSession(title: 'Garden current chat'),
        ))!;
        if (panel) {
          await toggleAgentPanel(t.element(find.byType(HomeScreen)));
          await settle(t);
        } else {
          await openAgentScreen(t);
        }
        store.sessions = [
          currentChat,
          referenced,
          for (var i = 0; i < 4; i++)
            AgentSession(
              id: 'extra-$i',
              title: 'Garden extra $i',
              createdAt: referenced.createdAt,
              updatedAt: referenced.updatedAt,
            ),
        ];
        await t.tap(key('agent-composer'));
        await t.enterText(key('agent-composer'), '@Garden');
        await settle(t);
        expect(key('mention-chat-${referenced.id}'), findsOneWidget);
        expect(key('mention-chat-${currentChat.id}'), findsNothing);
        expect(key('mention-chat-extra-2'), findsOneWidget);
        expect(key('mention-chat-extra-3'), findsNothing);
        // Matching pages precede chats in the combined menu.
        final pageItem = key('mention-page-${current.pages['Garden plan']}');
        expect(pageItem, findsOneWidget);
        expect(
          t.getTopLeft(pageItem).dy,
          lessThan(t.getTopLeft(key('mention-chat-${referenced.id}')).dy),
        );
        await tapAndSettle(t, key('mention-chat-${referenced.id}'));
        expect(key('agent-context-chat-${referenced.id}'), findsOneWidget);
        expect(
          t.widget<EditableText>(key('agent-composer')).controller.text,
          '@Garden chat ',
        );
        await tapAndSettle(t, key('agent-send'));
        await settleUntil(
          t,
          () =>
              !store.running &&
              (store.state?.messages.any((m) => m.role == 'assistant') ??
                  false),
        );
        final message = store.state!.messages.firstWhere(
          (m) => m.role == 'user',
        );
        expect(message.context?.chats.single.id, referenced.id);
        expect(message.context?.chats.single.title, 'Garden chat');
        expect(message.text, '@Garden chat');
        final chip = key('agent-msg-${message.id}-chat-${referenced.id}');
        expect(chip, findsOneWidget);
        await tapAndSettle(t, chip);
        expect(store.openId, referenced.id);
        expect(key(panel ? 'agent-panel' : 'agent-screen'), findsOneWidget);
      },
    );
  }
}
