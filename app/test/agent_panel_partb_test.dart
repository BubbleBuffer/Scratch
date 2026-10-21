import 'dart:convert';
import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/state/editor_focus.dart';
import 'package:notes_app/state/ui_state.dart';

import 'agent_test.dart' show openAgentStore, openAgentScreen;
import 'harness.dart';
import 'sync_helpers.dart' show real, key;

BuildContext homeContext(WidgetTester t) => t.element(find.byType(HomeScreen));

void main() {
  setUpAll(initCore);

  test('agent dock state persists and width is clamped per vault', () {
    final dir = Directory.systemTemp.createTempSync('notes-panel-state-');
    try {
      final file = File('${dir.path}/ui.json');
      final state = UiState(file: file, vaultRoot: 'vault-a')
        ..agentPanelOpen = true
        ..agentPanelWidth = 470
        ..agentSessionId = 'chat-a';
      state.flush();
      final restored = UiState.load(file, 'vault-a');
      expect(restored.agentPanelOpen, isTrue);
      expect(restored.agentPanelWidth, 470);
      expect(restored.agentSessionId, 'chat-a');
      final json = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
      json['agentPanelWidth'] = 900;
      file.writeAsStringSync(jsonEncode(json));
      expect(UiState.load(file, 'vault-a').agentPanelWidth, 560);
      final other = UiState.load(file, 'vault-b');
      expect(other.agentPanelOpen, isFalse);
      expect(other.agentSessionId, isNull);
      expect(other.agentPanelWidth, 400);
      state.dispose();
      restored.dispose();
      other.dispose();
    } finally {
      dir.deleteSync(recursive: true);
    }
  });

  test(
    'explicit Ask snapshot survives clearing its originating editor once',
    () {
      final focus = EditorFocus();
      focus.update(
        pageId: 'page-a',
        pageTitle: 'Garden',
        blockIds: ['b1', 'b2'],
        text: 'Selected text',
      );
      focus.requestComposer();
      focus.clear('page-a');
      final selection = focus.takeComposerSelection();
      expect(selection?.pageId, 'page-a');
      expect(selection?.blockIds, ['b1', 'b2']);
      expect(selection?.text, 'Selected text');
      expect(focus.takeComposerSelection(), isNull);
      focus.dispose();
    },
  );

  appTest('Ctrl+J toggles a single dock and drag resizes it', (t) async {
    await pumpApp(t, mobile: false);
    final store = await openAgentStore(t);
    await real(t, () => store.createSession(title: 'Dock chat'));
    FocusManager.instance.primaryFocus?.unfocus();
    await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.keyJ);
    await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await settle(t);
    expect(key('agent-panel'), findsOneWidget);
    expect(current.app.ui.agentSessionId, store.openId);
    expect(t.getSize(key('agent-panel')).width, 400);
    await t.drag(
      key('agent-panel-resize'),
      const Offset(-90, 0),
      kind: PointerDeviceKind.mouse,
    );
    await settle(t);
    expect(current.app.ui.agentPanelWidth, closeTo(490, 2));
    await t.drag(
      key('agent-panel-resize'),
      const Offset(-400, 0),
      kind: PointerDeviceKind.mouse,
    );
    await settle(t);
    expect(current.app.ui.agentPanelWidth, 560);
    await tapAndSettle(t, key('agent-panel-close'));
    expect(key('agent-panel'), findsNothing);
  });

  appTest('full-screen page handoff retains the chat beside one editor', (
    t,
  ) async {
    await pumpApp(t, mobile: false);
    final store = await openAgentStore(t);
    await real(t, () => store.createSession(title: 'Garden discussion'));
    final chatId = store.openId!;
    await openAgentScreen(t);
    final ctx = t.element(key('agent-screen'));
    final pageId = current.pages['Garden plan']!;
    await openAgentPage(ctx, pageId, 'Garden plan', sessionId: chatId);
    await settle(t);
    expect(key('agent-screen'), findsNothing);
    expect(key('agent-panel'), findsOneWidget);
    expect(
      find.byWidgetPredicate((w) => w is PageScreen && w.pageId == pageId),
      findsOneWidget,
    );
    expect(store.openId, chatId);
    expect(current.app.ui.agentSessionId, chatId);
  });

  appTest('Ask agent keeps the selection and requests the composer', (t) async {
    await pumpApp(t, mobile: false);
    await openAgentStore(t);
    final focus = EditorFocus.forApp(current.app);
    focus.update(
      pageId: current.pages['Recipes']!,
      pageTitle: 'Recipes',
      blockIds: ['first', 'second'],
      text: 'Selected recipe',
    );
    await askAgent(homeContext(t));
    await settle(t);
    expect(current.app.ui.agentPanelOpen, isTrue);
    expect(focus.composerRequest, 1);
    expect(focus.text, 'Selected recipe');
    expect(key('agent-panel'), findsOneWidget);
    expect(
      t.widget<EditableText>(key('agent-composer')).focusNode.hasFocus,
      isTrue,
    );
  });

  appTest('UI actions navigate only for a currently visible chat', (t) async {
    await pumpApp(t, mobile: false);
    final store = await openAgentStore(t);
    await real(t, () => store.createSession());
    await toggleAgentPanel(homeContext(t));
    await settle(t);
    final pageId = current.pages['Budget']!;
    store.debugUiAction(
      AgentUiAction(
        sessionId: 'other-chat',
        runId: 'r',
        action: 'open_page',
        pageId: pageId,
        blockIds: const [],
      ),
    );
    await settle(t);
    expect(current.app.ui.lastPage, isNull);
    store.debugUiAction(
      AgentUiAction(
        sessionId: store.openId!,
        runId: 'r',
        action: 'open_page',
        pageId: pageId,
        blockIds: const [],
      ),
    );
    await settle(t);
    expect(current.app.ui.lastPage, pageId);
    await tapAndSettle(t, key('agent-panel-close'));
    store.debugUiAction(
      AgentUiAction(
        sessionId: store.openId!,
        runId: 'r',
        action: 'open_page',
        pageId: current.pages['Recipes']!,
        blockIds: const [],
      ),
    );
    await settle(t);
    expect(current.app.ui.lastPage, pageId);
  });

  appTest('mobile UI action shows Agent notice and waits for manual Open', (
    t,
  ) async {
    await pumpApp(t, mobile: false);
    final store = await openAgentStore(t);
    await real(t, () => store.createSession());
    await openAgentScreen(t);
    t.view.physicalSize = const Size(700, 900);
    await settle(t);
    final pageId = current.pages['Budget']!;
    store.debugUiAction(
      AgentUiAction(
        sessionId: store.openId!,
        runId: 'r',
        action: 'open_page',
        pageId: pageId,
        blockIds: const [],
      ),
    );
    await settle(t);
    expect(key('agent-page-notice'), findsOneWidget);
    expect(find.text('Agent: Budget'), findsOneWidget);
    expect(current.app.ui.lastPage, isNull);
    await tapAndSettle(t, key('agent-page-notice-open'));
    expect(current.app.ui.lastPage, pageId);
  });

  appTest('mobile Agent notice names an untitled page by its first body line', (
    t,
  ) async {
    await pumpApp(t, mobile: false);
    final store = await openAgentStore(t);
    await real(t, () => store.createSession());
    await openAgentScreen(t);
    t.view.physicalSize = const Size(700, 900);
    await settle(t);
    final pageId = await core(t, (v) async {
      final id = await v.createPage();
      await v.apply(id, [
        BOp(
          op: OpKind.insert,
          id: newId(),
          kind: Kinds.paragraph,
          text: 'Groceries for Sunday',
          marks: const [],
          checked: false,
        ),
      ]);
      return id;
    });
    store.debugUiAction(
      AgentUiAction(
        sessionId: store.openId!,
        runId: 'r',
        action: 'open_page',
        pageId: pageId,
        blockIds: const [],
      ),
    );
    await settleUntil(t, () => key('agent-page-notice').evaluate().isNotEmpty);
    expect(find.text('Agent: Groceries for Sunday'), findsOneWidget);
    expect(find.text('Agent: Untitled'), findsNothing);
    expect(current.app.ui.lastPage, isNull);
  });
}
