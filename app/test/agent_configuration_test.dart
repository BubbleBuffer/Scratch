import 'dart:io';

import 'package:agent_kit/agent_kit.dart' show KitChoice;
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
      expect(settings.legacyAgentModel, 'codex/model');
      expect(settings.agentModelOverrides, isEmpty,
          reason: 'old values are not device overrides');
      expect(restored.legacyAgentModel, 'codex/model');
    },
  );

  appTest(
    'the chat picker lists the models the host reports, not stored hidden or custom ids',
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
      final picker = t.widget<SelectButton<KitChoice>>(key('agent-model'));
      expect(picker.options.map((c) => c.id), ['codex/shown', 'codex/hidden', 'codex/current']);
      expect(picker.value.id, 'codex/current');
      expect(store.modelLabel('claude-code/opus'), 'Opus (latest)');
      expect(store.modelLabel('claude-code/claude-fable-5-1'), 'Fable 5.1');
      expect(store.modelLabel('codex/shown'), 'Shown');
    },
  );

  appTest('the settings page has no model switches or custom ids', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final file = File('${tv.dir.path}/model-settings.json');
    final settings = AiSettings.load(file);
    final fake = fakeAgentSettings();
    settings
      ..agentEnabled = fake.agentEnabled
      ..agentModelOverrides.addAll(fake.agentModelOverrides)
      ..agentBunPath = fake.agentBunPath
      ..agentClaudeCodeDir = fake.agentClaudeCodeDir
      // Left by an older build: not applied any more.
      ..agentHiddenModels = {'claude-code/claude-opus-5-5'}
      ..agentCustomModels = ['claude-code/claude-opus-4-7'];
    final store = await openAgentStore(t, settings: settings);
    final ai = await aiStore(t, settings: settings);
    try {
      store.models = const [
        AgentModel(id: 'codex/shown', name: 'Shown'),
        AgentModel(id: 'codex/hidden', name: 'Hidden'),
      ];
      expect(store.shownModels(), ['codex/shown', 'codex/hidden']);
      await tapAndSettle(t, key('sidebar-settings'));
      await tapAndSettle(t, key('settings-nav-ai'));
      expect(key('agent-model-visible-claude-code/claude-opus-5-5'), findsNothing);
      expect(key('agent-model-add-field'), findsNothing);
      expect(key('agent-model-add'), findsNothing);
      expect(key('agent-model-remove-claude-code/claude-opus-4-7'), findsNothing);
      // The host's own list shows; a hidden or custom id from an older build is not applied.
      expect(store.shownModels(), contains('claude-code/claude-opus-5-5'));
      expect(store.shownModels(), isNot(contains('claude-code/claude-opus-4-7')));
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
