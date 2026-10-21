import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;
import 'package:notes_app/state/editor_focus.dart';

import 'harness.dart';
import 'agent_test.dart' show openAgentStore;

/// Observe the handoff before composer focus changes the live editor selection.
class ComposerHandoff {
  ComposerHandoff(this.focus) : request = focus.composerRequest {
    focus.addListener(_capture);
  }

  final EditorFocus focus;
  int request;
  AgentContextSelection? selection;

  void _capture() {
    if (request == focus.composerRequest) return;
    request = focus.composerRequest;
    final pageId = focus.pageId;
    if (pageId == null) return;
    selection = AgentContextSelection(
      pageId: pageId,
      pageTitle: focus.pageTitle,
      blockIds: List.of(focus.blockIds),
      text: focus.text,
    );
  }

  void dispose() => focus.removeListener(_capture);
}

PageEditorState editor(WidgetTester tester, String id) =>
    tester.state<PageEditorState>(
      find.byWidgetPredicate(
        (w) => w is PageScreen && w.pageId == id && w.snapshot == null,
      ),
    );

Future<void> open(WidgetTester tester, String id, {bool stack = false}) async {
  final sheets = nav.PageStackScope.of(tester.element(find.byType(HomeScreen)));
  final context = sheets.topId == null
      ? tester.element(find.byType(HomeScreen))
      : editor(tester, sheets.topId!).context;
  unawaited(
    nav.openPage(context, id, current.app.pages.meta(id)!.title, stack: stack),
  );
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  appTest('multiple agent references flash together for three seconds', (
    t,
  ) async {
    final vault = await pumpApp(t, mobile: false);
    final id = vault.pages['Recipes']!;
    await open(t, id);
    final ed = editor(t, id);
    final ids = ed.doc!.flat.take(2).map((n) => n.id).toList();
    nav.PageStackScope.of(ed.context).highlightBlocks[id]!(ids);
    await t.pump();
    expect(ed.highlightedBlocks, unorderedEquals(ids));
    await t.pump(const Duration(seconds: 2));
    expect(ed.highlightedBlocks, unorderedEquals(ids));
    await t.pump(const Duration(seconds: 1));
    expect(ed.highlightedBlocks, isEmpty);
  });

  appTest('only the top editor publishes selection and current title', (
    t,
  ) async {
    final vault = await pumpApp(t, mobile: false);
    final a = vault.pages['Recipes']!, b = vault.pages['Budget']!;
    await open(t, a);
    final lower = editor(t, a);
    final focus = EditorFocus.forApp(vault.app);
    final n = lower.doc!.flat.firstWhere((n) => n.text.length >= 6);
    lower.focusBlock(n, 0);
    await settle(t);
    lower.ctl(n).selection = const TextSelection(
      baseOffset: 1,
      extentOffset: 5,
    );
    await t.pump();
    expect(focus.pageId, a);
    expect(focus.blockIds, [n.id]);
    expect(focus.text, lower.ctl(n).text.substring(1, 5));
    lower.titleCtl.text = 'Recipes for October';
    await t.pump();
    expect(focus.pageTitle, 'Recipes for October');

    await open(t, b, stack: true);
    expect(focus.pageId, b);
    lower.selectBlock(n);
    await t.pump();
    expect(focus.pageId, b);
    nav.goHome(editor(t, b).context);
    await settle(t);
    expect(focus.pageId, isNull);
    expect(focus.blockIds, isEmpty);
    expect(focus.text, isEmpty);
  });

  appTest('Ask agent block menu captures every selected block in order', (
    t,
  ) async {
    final vault = await pumpApp(t, mobile: false);
    await openAgentStore(t);
    final id = vault.pages['Recipes']!;
    await open(t, id);
    final ed = editor(t, id);
    final focus = EditorFocus.forApp(vault.app);
    final request = focus.composerRequest;
    final handoff = ComposerHandoff(focus);
    addTearDown(handoff.dispose);
    final nodes = ed.doc!.flat.take(2).toList();
    ed.selectBlock(nodes.last);
    ed.toggleSelected(nodes.first);
    ed.openBlockMenu(nodes.first, const Rect.fromLTWH(400, 150, 18, 24));
    await t.pump();
    await t.tap(find.byKey(const ValueKey('menu-ask-agent')));
    await t.pump();
    expect(vault.app.ui.agentPanelOpen, isTrue);
    expect(find.byKey(const ValueKey('agent-panel')), findsOneWidget);
    expect(focus.composerRequest, request + 1);
    expect(handoff.selection?.pageId, id);
    expect(handoff.selection?.blockIds, nodes.map((n) => n.id).toList());
    expect(handoff.selection?.text, nodes.map((n) => n.text).join('\n'));
    expect(find.byKey(const ValueKey('block-menu')), findsNothing);
  });

  appTest('Ask agent text toolbar captures only the selected substring', (
    t,
  ) async {
    final vault = await pumpApp(t, mobile: false);
    await openAgentStore(t);
    final id = vault.pages['Recipes']!;
    await open(t, id);
    final ed = editor(t, id);
    final focus = EditorFocus.forApp(vault.app);
    final request = focus.composerRequest;
    final handoff = ComposerHandoff(focus);
    addTearDown(handoff.dispose);
    final n = ed.doc!.flat.firstWhere((n) => n.text.length >= 8);
    ed.focusBlock(n, 0);
    await settle(t);
    const selection = TextSelection(baseOffset: 2, extentOffset: 7);
    ed.ctl(n).selection = selection;
    ed.onSelectionChanged(n, selection, SelectionChangedCause.keyboard);
    await t.pump();
    await t.pump();
    await t.tap(find.byKey(const ValueKey('fmt-ask-agent')));
    await t.pump();
    expect(vault.app.ui.agentPanelOpen, isTrue);
    expect(find.byKey(const ValueKey('agent-panel')), findsOneWidget);
    expect(focus.composerRequest, request + 1);
    expect(handoff.selection?.pageId, id);
    expect(handoff.selection?.blockIds, [n.id]);
    expect(handoff.selection?.text, n.text.substring(2, 7));
  });
}
