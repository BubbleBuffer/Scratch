// The Agent chat on the kit's `AgentChatPane`, end to end on the real core and
// the real agent bridge, with BubblePaw's fake Claude Code extension as the
// only model provider. Never calls a real AI.
// - a new chat is a pane draft: the first send creates it;
// - a page chip travels with the message and shows on the sent message;
// - a model pick (on a draft, then on a chat) reaches the store;
// - the approval bar decides, with the keys 1, 2 and 3;
// - the pane's own transcript draws Scratch's "Show code" under an activity
//   group (the hooks `scratchTranscriptActions` gives it).

import 'package:agent_kit/agent_kit.dart';
import 'package:agent_kit_core/agent_kit_core.dart' show KitChoice;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/addons/agent/kit_backend.dart';
import 'package:notes_app/addons/agent/kit_commands.dart';
import 'package:notes_app/addons/agent/kit_hooks.dart';
import 'package:notes_app/addons/agent/scratch_chat_pane.dart';
import 'package:notes_app/src/rust/api/agent.dart';
import 'package:notes_app/state/editor_focus.dart';
import 'package:notes_app/features/settings/controls.dart' show SelectButton;

import 'agent_chat_partb_test.dart' show showWidget;
import 'agent_test.dart'
    show fakeExtensionSkipReason, openAgentScreen, openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

String _draft(WidgetTester tester) =>
    tester.widget<EditableText>(key('agent-composer')).controller.text;

void main() {
  group('Agent kit pane', _paneTests, skip: fakeExtensionSkipReason);
}

void _paneTests() {
  setUpAll(initCore);

  appTest('a new chat is a draft; the first send creates it', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await openAgentScreen(tester);
    await settleUntil(
      tester,
      () => find.byType(AgentChatPane).evaluate().isNotEmpty,
    );
    expect(store.openId, isNull);
    expect(store.sessions, isEmpty);

    await tester.tap(key('agent-composer'));
    await tester.enterText(key('agent-composer'), 'Say ok');
    await tapAndSettle(tester, key('agent-send'));
    await settleUntil(
      tester,
      () =>
          (store.state?.messages.any(
                (m) => m.role == 'assistant' && m.content == 'ok',
              ) ??
              false) &&
          !store.running,
      timeout: const Duration(seconds: 30),
    );
    // The chat took its name from the first line and is the open one; the
    // pane stayed the same widget across the draft becoming a chat.
    expect(store.sessions.single.title, 'Say ok');
    expect(store.openId, store.sessions.single.id);
    expect(find.byType(AgentChatPane), findsOneWidget);
    expect(_draft(tester), isEmpty);
    expect(find.text('ok'), findsOneWidget);
  });

  appTest('a page chip travels with the message and shows on it', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    final garden = tv.pages['Garden plan']!;
    EditorFocus.forApp(
      tv.app,
    ).update(pageId: garden, pageTitle: 'Garden plan', blockIds: [], text: '');
    await showWidget(tester, ScratchChatPane(store: store, panel: true));
    await settleUntil(
      tester,
      () => find.byType(AgentChatPane).evaluate().isNotEmpty,
    );
    expect(key('agent-context-page'), findsOneWidget);

    await tester.tap(key('agent-composer'));
    await tester.enterText(key('agent-composer'), 'Say ok');
    await tapAndSettle(tester, key('agent-send'));
    await settleUntil(
      tester,
      () =>
          (store.state?.messages.any((m) => m.role == 'assistant') ?? false) &&
          !store.running,
      timeout: const Duration(seconds: 30),
    );
    final message = store.state!.messages.firstWhere((m) => m.role == 'user');
    expect(message.context!.pages.map((p) => p.id), [garden]);
    expect(key('agent-msg-${message.id}-page-$garden'), findsOneWidget);
  });

  appTest('a model pick reaches the store, on a draft and on a chat', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    store.models = const [
      AgentModel(id: 'codex/one', name: 'One'),
      AgentModel(id: 'codex/two', name: 'Two'),
    ];
    store.pickModel(null, 'codex/one');
    await openAgentScreen(tester);
    await settleUntil(
      tester,
      () => find.byType(AgentChatPane).evaluate().isNotEmpty,
    );
    SelectButton<KitChoice> picker() =>
        tester.widget<SelectButton<KitChoice>>(key('agent-model'));
    expect(picker().value.id, 'codex/one');

    // On the draft the store keeps the pick.
    picker().onChanged!(picker().options.last);
    await settle(tester);
    expect(store.modelOf(null), 'codex/two');
    expect(picker().value.id, 'codex/two');

    // On a chat the pick goes through the pane and the backend.
    final chat = (await real(
      tester,
      () => store.createSession(title: 'Pick'),
    ))!;
    await settle(tester);
    picker().onChanged!(picker().options.first);
    await settleUntil(tester, () => store.modelOf(chat.id) == 'codex/one');
    expect(picker().value.id, 'codex/one');
    expect(store.error, isNull);
  });

  appTest(
    'the approval bar decides, and 1 answers while the composer is empty',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final s = (await real(
        tester,
        () => store.createSession(title: 'Approvals'),
      ))!;
      await settleUntil(
        tester,
        () => find.byType(AgentChatPane).evaluate().isNotEmpty,
      );
      AgentApproval approval(String id) => AgentApproval(
        id: id,
        sessionId: s.id,
        runId: 'run',
        callId: 'call-$id',
        tool: 'delete_page',
        argumentsJson: '{"page":"Garden plan"}',
        state: 'pending',
        createdAt: '2026-10-06T11:20:00Z',
      );
      store.debugAddApproval(approval('a1'));
      await settle(tester);
      expect(key('agent-approval-a1'), findsOneWidget);
      // The pane itself draws no second bar for it.
      expect(find.text('Needs approval'), findsOneWidget);
      await tapAndSettle(tester, find.text('Approve'));
      expect(key('agent-approval-a1'), findsNothing);

      store.debugAddApproval(approval('a2'));
      await settle(tester);
      expect(key('agent-approval-a2'), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.digit1);
      await settle(tester);
      expect(key('agent-approval-a2'), findsNothing);
    },
  );

  appTest('the pane draws Show code under an activity group', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    final s = (await real(tester, () => store.createSession(title: 'Script')))!;
    final run = await real(
      tester,
      () => store.runScript(s.id, 'Count pages', 'Math.max(1,\n  3)'),
    );
    expect(run?.status, 'completed');
    final agent = (await real(tester, store.ensureOpen))!;
    final backend = ScratchKitBackend(
      NotesAgentSource(agent, events: store.eventsFor(agent)),
      commands: StoreKitCommands(store),
    );
    // The pane's own list (no transcript slot), with Scratch's hooks.
    await showWidget(
      tester,
      BbThemeScope(
        brightness: Brightness.light,
        child: Builder(
          builder: (context) => AgentChatPane(
            backend: backend,
            chatId: s.id,
            transcriptActions: scratchTranscriptActions(context, store),
          ),
        ),
      ),
    );
    await settleUntil(
      tester,
      () => find.textContaining('other call').evaluate().isNotEmpty,
    );
    // Closed: no code control. Open the group, then the code.
    expect(find.text('Show code'), findsNothing);
    await tapAndSettle(tester, find.textContaining('other call').first);
    expect(find.text('Show code'), findsOneWidget);
    await tapAndSettle(tester, find.text('Show code'));
    final call = store.state!.messages.expand((m) => m.toolCalls).single;
    final cell = 'agent-cell-${call.id}';
    expect(key(cell), findsOneWidget);
    await tapAndSettle(tester, key('$cell-toggle'));
    expect(textOf(tester, '$cell-code'), 'Math.max(1,\n  3)');
    expect(textOf(tester, '$cell-result'), '3');
  });
}
