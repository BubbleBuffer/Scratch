import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/app_shell.dart';
import 'package:notes_app/src/rust/api/agent.dart' show AgentEvent;
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/state/editor_focus.dart';
import 'package:notes_app/ui/ui.dart';

import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

const _at = '2026-10-09T12:00:00Z';
AgentEdit edit(
  String id, {
  bool undone = false,
  bool undoable = true,
  String kind = 'edited',
  String sessionId = 'chat',
  String pageTitle = 'Garden plan',
  String? pageId,
}) => AgentEdit(
  id: id,
  sessionId: sessionId,
  runId: 'run',
  callId: 'call',
  at: _at,
  pageId: pageId ?? current.pages['Garden plan']!,
  pageTitle: pageTitle,
  kind: kind,
  added: 13,
  removed: 1,
  changedBlocks: const ['changed'],
  undone: undone,
  undoable: undoable,
  hunks: [
    AgentDiffHunk(
      oldStart: 1,
      newStart: 1,
      lines: [
        const AgentDiffLine(tag: '-', text: 'old line'),
        for (var i = 0; i < 13; i++)
          AgentDiffLine(tag: '+', text: 'new line $i'),
      ],
    ),
  ],
);

AgentSessionState snapshot({
  required AgentSession session,
  String status = 'completed',
  String pageTitle = 'Garden plan',
  String? untitledPageId,
}) => AgentSessionState(
  session: session,
  seq: 0,
  liveSeen: 0,
  streamingText: '',
  pendingApprovals: const [],
  runs: [
    AgentRunRecord(
      id: 'run',
      sessionId: session.id,
      model: 'test',
      status: status,
      startedAt: _at,
      endedAt: status == 'running' ? null : '2026-10-09T12:01:00Z',
      error: status == 'failed' ? 'Connection failed' : null,
    ),
  ],
  messages: [
    AgentMessage(
      id: 'user',
      role: 'user',
      content: 'Human text\n<private context payload>',
      text: 'Human text',
      context: AgentMessageContext(
        pages: [
          AgentContextPage(
            id: untitledPageId ?? current.pages['Garden plan']!,
            title: untitledPageId == null ? 'Garden plan' : '',
          ),
        ],
        selection: AgentContextSelection(
          pageId: current.pages['Garden plan']!,
          pageTitle: 'Garden plan',
          blockIds: const ['changed'],
          text: 'selected text',
        ),
      ),
      toolCalls: const [],
      isError: false,
      createdAt: '2026-10-09T11:59:59Z',
    ),
    const AgentMessage(
      id: 'assistant',
      role: 'assistant',
      content: '**Finished**',
      text: '**Finished**',
      isError: false,
      createdAt: _at,
      toolCalls: [
        AgentToolCall(
          id: 'call',
          name: 'js',
          argumentsJson: '{"code":"return 42"}',
        ),
      ],
    ),
    const AgentMessage(
      id: 'result',
      role: 'tool',
      content: '42',
      text: '42',
      toolCalls: [],
      toolCallId: 'call',
      isError: false,
      createdAt: _at,
    ),
  ],
  activity: const [
    AgentToolActivity(
      runId: 'run',
      callId: 'call',
      order: 1,
      name: 'search_notes',
      summary: 'Searched notes “garden”',
      isError: false,
    ),
    AgentToolActivity(
      runId: 'run',
      callId: 'read',
      order: 2,
      name: 'read_page',
      summary: 'Read Garden plan',
      isError: false,
    ),
    AgentToolActivity(
      runId: 'run',
      callId: 'bad',
      order: 3,
      name: 'exec',
      summary: 'Command failed',
      isError: true,
    ),
  ],
  edits: [
    edit(
      'edit',
      sessionId: session.id,
      pageTitle: pageTitle,
      pageId: untitledPageId,
    ),
  ],
);

Future<void> showWidget(
  WidgetTester tester,
  Widget child, {
  bool quiet = true,
}) async {
  await tester.pumpWidget(
    NotesShell(
      forceDark: false,
      navigatorKey: GlobalKey<NavigatorState>(),
      wrap: (child) => AppScope(state: current.app, child: child),
      home: Center(child: SizedBox(width: 480, height: 650, child: child)),
    ),
  );
  if (quiet) {
    await settle(tester);
  } else {
    // Live status intentionally animates continuously; only advance the frames
    // needed to lay out the fixture instead of waiting for it to become quiet.
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
  }
}

Future<void> revealAndTap(WidgetTester tester, Finder target) async {
  await Scrollable.ensureVisible(tester.element(target), alignment: 0.5);
  await settle(tester);
  await tapAndSettle(tester, target);
}

void expectSendAtRight(WidgetTester tester) {
  final frame = find
      .ancestor(of: key('agent-composer'), matching: find.byType(FieldFrame))
      .first;
  final frameBounds = tester.getRect(frame);
  final sendBounds = tester.getRect(key('agent-send'));
  expect(frameBounds.right - sendBounds.right, inInclusiveRange(0, 12));
}

AgentSessionState chronologicalFixture({
  bool live = false,
  bool queued = false,
}) => AgentSessionState(
  session: const AgentSession(
    id: 'chronology',
    title: 'Chronology',
    createdAt: _at,
    updatedAt: _at,
  ),
  seq: 0,
  liveSeen: 0,
  streamingText: '',
  pendingApprovals: const [],
  activity: const [],
  edits: const [],
  runs: [
    AgentRunRecord(
      id: 'first',
      sessionId: 'chronology',
      model: 'test',
      status: live ? 'running' : 'completed',
      startedAt: '2026-10-09T14:00:00+02:00',
    ),
    if (queued)
      const AgentRunRecord(
        id: 'next',
        sessionId: 'chronology',
        model: 'test',
        status: 'running',
        startedAt: '2026-10-09T12:00:00.004Z',
      ),
  ],
  messages: [
    const AgentMessage(
      id: 'user',
      role: 'user',
      content: 'Question',
      text: 'Question',
      toolCalls: [],
      isError: false,
      createdAt: '2026-10-09T12:00:00.001Z',
    ),
    if (!live)
      const AgentMessage(
        id: 'answer',
        role: 'assistant',
        content: 'Answer',
        text: 'Answer',
        toolCalls: [],
        isError: false,
        createdAt: '2026-10-09T12:00:00.002Z',
      ),
    if (queued)
      const AgentMessage(
        id: 'queued',
        role: 'user',
        content: 'Follow up',
        text: 'Follow up',
        toolCalls: [],
        isError: false,
        createdAt: '2026-10-09T12:00:00.003Z',
      ),
  ],
);

void main() {
  setUpAll(initCore);

  test('turns follow their user when run creation precedes message append', () {
    final items = agentTurns(chronologicalFixture());
    expect(items.map((item) => item.id), ['user', 'first']);
    expect((items.last as AgentTurn).texts.single.text, 'Answer');
  });

  test(
    'live Working follows its initiating user before any assistant message',
    () {
      final items = agentTurns(chronologicalFixture(live: true));
      expect(items.map((item) => item.id), ['user', 'first']);
      expect((items.last as AgentTurn).live, isTrue);
    },
  );

  test('queued user retains chronology before its empty next run', () {
    final items = agentTurns(chronologicalFixture(queued: true));
    expect(items.map((item) => item.id), ['user', 'first', 'queued', 'next']);
  });

  test('work groups row upserts with page edits', () {
    const turn = AgentTurn(
      'row-work',
      texts: [],
      cells: [],
      edits: [],
      activity: [
        AgentToolActivity(
          runId: 'row-work',
          callId: 'row-call',
          order: 1,
          name: 'upsert_row',
          summary: 'Updated task row',
          isError: false,
        ),
      ],
    );
    expect(workSummary(turn), 'Edited 1 page');
  });

  test('work groups notes and native file tools by their targets', () {
    final turn = AgentTurn(
      'native-work',
      texts: const [],
      cells: const [],
      edits: const [],
      activity: [
        for (final (order, name) in [
          'search_notes',
          'search',
          'read_page',
          'read',
          'write',
          'edit',
          'list',
        ].indexed)
          AgentToolActivity(
            runId: 'native-work',
            callId: 'call-$order',
            order: order,
            name: name,
            summary: name,
            isError: false,
          ),
      ],
    );
    expect(
      workSummary(turn),
      'Searched 2× · Read 1 page · Read 1 file · Edited 2 files · Listed 1×',
    );
  });

  appTest('live progress labels distinguish notes and native file tools', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final store = AgentStore.of(current.app);
    const session = AgentSession(
      id: 'progress',
      title: 'Progress labels',
      createdAt: _at,
      updatedAt: _at,
    );
    store.debugSetSnapshot(snapshot(session: session, status: 'running'));
    for (final entry in {
      'search_notes': 'Searching notes',
      'search': 'Searching files',
      'read_page': 'Reading a page',
      'read': 'Reading a file',
      'write': 'Editing a file',
      'edit': 'Editing a file',
      'list': 'Listing files',
    }.entries) {
      store.debugEvent(
        AgentEvent(
          kind: 'tool_started',
          sequence: 0,
          durable: false,
          sessionId: session.id,
          runId: 'run',
          createdAt: _at,
          toolName: entry.key,
          isError: false,
          skipped: 0,
          json: '{}',
        ),
      );
      expect(store.latestActivitySummary, entry.value);
    }
  });

  appTest(
    'minimum dock width contains long edit titles and keeps edit controls usable',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final session = (await real(
        tester,
        () => store.createSession(title: 'Narrow dock'),
      ))!;
      const title =
          'A very long garden planning title with planting schedules, seed inventories and seasonal reminders';
      await showWidget(
        tester,
        Center(
          child: SizedBox(
            width: 320,
            child: AgentChat(store: store, panel: true),
          ),
        ),
      );
      store.debugSetSnapshot(snapshot(session: session, pageTitle: title));
      await settle(tester);
      expect(tester.takeException(), isNull);
      expectSendAtRight(tester);
      final row = key('agent-edit-edit');
      final rowBounds = tester.getRect(row);
      for (final suffix in ['toggle', 'open', 'undo']) {
        final control = key('agent-edit-edit-$suffix');
        expect(control.hitTestable(), findsOneWidget);
        final bounds = tester.getRect(control);
        expect(bounds.left, greaterThanOrEqualTo(rowBounds.left));
        expect(bounds.right, lessThanOrEqualTo(rowBounds.right));
      }
      expect(
        tester.widget<TextBtn>(key('agent-edit-edit-open')).onTap,
        isNotNull,
      );
      expect(
        tester.widget<TextBtn>(key('agent-edit-edit-undo')).onTap,
        isNotNull,
      );
      await revealAndTap(tester, key('agent-edit-edit-toggle'));
      expect(key('agent-edit-edit-line-0'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  appTest('turns show human text and compact edits, then work and gated code', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    final session = (await real(
      tester,
      () => store.createSession(title: 'Chat fixture'),
    ))!;
    await showWidget(
      tester,
      Center(
        child: SizedBox(
          width: 400,
          child: AgentChat(store: store, panel: true),
        ),
      ),
    );
    store.debugSetSnapshot(snapshot(session: session));
    await settle(tester);
    expect(find.text('Human text'), findsOneWidget);
    expectSendAtRight(tester);
    expect(find.textContaining('private context payload'), findsNothing);
    expect(
      key('agent-msg-user-page-${current.pages['Garden plan']}'),
      findsOneWidget,
    );
    expect(key('agent-msg-user-selection'), findsOneWidget);
    final bubble = tester.getRect(key('agent-msg-user'));
    final pageChip = tester.getRect(
      key('agent-msg-user-page-${current.pages['Garden plan']}'),
    );
    final selectionChip = tester.getRect(key('agent-msg-user-selection'));
    expect(pageChip.width, lessThan(bubble.width * 0.75));
    expect(selectionChip.width, lessThan(bubble.width * 0.75));
    expect((pageChip.center.dy - selectionChip.center.dy).abs(), lessThan(1));
    expect(
      pageChip.bottom,
      lessThan(tester.getRect(find.text('Human text')).top),
    );
    expect(
      bubble.bottom,
      lessThanOrEqualTo(tester.getRect(key('agent-turn-run')).top),
    );
    expect(find.text('Edited Garden plan'), findsOneWidget);
    final editBounds = tester.getRect(key('agent-edit-edit'));
    final open = tester.getRect(key('agent-edit-edit-open'));
    final undo = tester.getRect(key('agent-edit-edit-undo'));
    final label = tester.getRect(key('agent-edit-edit-toggle'));
    expect(open.width, lessThan(editBounds.width / 2));
    expect(undo.width, lessThan(editBounds.width / 2));
    expect((open.center.dy - undo.center.dy).abs(), lessThan(1));
    expect((open.center.dy - label.center.dy).abs(), lessThan(1));
    expect(find.text('+13'), findsOneWidget);
    expect(find.text('−1'), findsOneWidget);
    expect(key('agent-edit-edit-line-0'), findsNothing);
    expect(key('agent-cell-call'), findsNothing);
    expect(
      find.text(
        'Searched 1× · Read 1 page · Ran 1 command · 1 failed',
        findRichText: true,
      ),
      findsOneWidget,
    );
    await tapAndSettle(tester, key('agent-edit-edit-toggle'));
    expect(key('agent-edit-edit-line-11'), findsOneWidget);
    expect(key('agent-edit-edit-line-12'), findsNothing);
    final removed = tester.widget<Text>(key('agent-edit-edit-line-0'));
    expect(removed.style!.decoration, TextDecoration.lineThrough);
    await revealAndTap(tester, key('agent-edit-edit-all'));
    expect(key('agent-edit-edit-line-13'), findsOneWidget);
    await revealAndTap(tester, key('agent-work-run-toggle'));
    expect(find.text('Command failed'), findsOneWidget);
    expect(key('agent-cell-call'), findsNothing);
    await revealAndTap(tester, key('agent-work-run-code'));
    expect(key('agent-cell-call'), findsOneWidget);
    await revealAndTap(tester, key('agent-cell-call-toggle'));
    expect(textOf(tester, 'agent-cell-call-code'), 'return 42');
    expect(textOf(tester, 'agent-cell-call-result'), '42');
  });

  appTest(
    'undo becomes disabled Undone, refusal stays inline, unavailable undo is hidden',
    (tester) async {
      await pumpApp(tester, mobile: false);
      var calls = 0;
      await showWidget(
        tester,
        Column(
          children: [
            AgentEditRow(
              edit: edit('success'),
              onUndo: () async {
                calls++;
                return edit('success', undone: true);
              },
            ),
            AgentEditRow(
              edit: edit('refused'),
              onUndo: () async =>
                  throw Exception('Page changed since this edit'),
            ),
            AgentEditRow(
              edit: edit('unavailable', undoable: false),
              onUndo: () async => throw StateError('must not be called'),
            ),
            AgentEditRow(
              edit: edit('created', kind: 'created'),
              onUndo: () async => edit('created', undone: true),
            ),
            AgentEditRow(
              edit: edit('trashed', kind: 'trashed'),
              onUndo: () async => edit('trashed', undone: true),
            ),
          ],
        ),
      );
      expect(key('agent-edit-unavailable-undo'), findsNothing);
      expect(find.text('Created Garden plan'), findsOneWidget);
      expect(find.text('Moved Garden plan to trash'), findsOneWidget);
      expect(find.text('+13'), findsNWidgets(3));
      await tapAndSettle(tester, key('agent-edit-success-undo'));
      expect(find.text('Undone'), findsOneWidget);
      expect(
        tester.widget<TextBtn>(key('agent-edit-success-undo')).onTap,
        isNull,
      );
      expect(calls, 1);
      await tapAndSettle(tester, key('agent-edit-refused-undo'));
      expect(
        textOf(tester, 'agent-edit-refused-refusal'),
        'Page changed since this edit',
      );
    },
  );

  appTest(
    'live turn reports Working, hides code, and preserves stop/error states',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final session = (await real(
        tester,
        () => store.createSession(title: 'Run fixture'),
      ))!;
      // Complete real view registration and catch-up before installing the
      // synthetic live run, which intentionally never becomes animation-idle.
      await showWidget(tester, AgentChat(store: store));
      store.debugSetSnapshot(snapshot(session: session, status: 'running'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      expect(textOf(tester, 'agent-working'), 'Working · Command failed');
      expect(key('agent-stop'), findsOneWidget);
      await tester.tap(key('agent-work-run-toggle'));
      await tester.pump(const Duration(milliseconds: 50));
      expect(key('agent-work-run-code'), findsNothing);
      store.debugSetSnapshot(snapshot(session: session, status: 'cancelled'));
      await settle(tester);
      expect(find.text('Stopped.'), findsOneWidget);
      expect(key('agent-working'), findsNothing);
      store.debugSetSnapshot(snapshot(session: session, status: 'failed'));
      await settle(tester);
      expect(find.text('Connection failed'), findsOneWidget);
    },
  );

  appTest(
    'panel context is removable, follows page changes and clears with editor',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final focus = EditorFocus.forApp(current.app);
      final garden = current.pages['Garden plan']!;
      focus.update(
        pageId: garden,
        pageTitle: 'Garden plan',
        blockIds: const ['one', 'two'],
        text: 'selected',
      );
      await showWidget(tester, AgentComposer(store: store, panel: true));
      expect(key('agent-context-page'), findsOneWidget);
      expect(find.text('Selection · 2 blocks'), findsOneWidget);
      await tapAndSettle(tester, key('agent-context-page-remove'));
      await tapAndSettle(tester, key('agent-context-selection-remove'));
      focus.update(
        pageId: garden,
        pageTitle: 'Garden plan renamed',
        blockIds: const ['one', 'two'],
        text: 'selected',
      );
      await settle(tester);
      expect(key('agent-context-page'), findsNothing);
      expect(key('agent-context-selection'), findsNothing);
      final recipes = current.pages['Recipes']!;
      focus.update(pageId: recipes, pageTitle: 'Recipes');
      await settle(tester);
      expect(find.text('Recipes'), findsOneWidget);
      focus.clear(recipes);
      await settle(tester);
      expect(key('agent-context-page'), findsNothing);
    },
  );

  appTest(
    'panel sends its current page and quoted selection as typed context',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Selection context'));
      final garden = tv.pages['Garden plan']!;
      await core(
        tester,
        (vault) => vault.apply(garden, [
          BOp(
            op: OpKind.insert,
            id: newId(),
            kind: Kinds.paragraph,
            text: 'Selected garden passage for the agent.',
            marks: const [],
            checked: false,
          ),
        ]),
      );
      final page = await real(tester, () => tv.vault.loadPage(garden));
      final block = page.blocks.firstWhere((b) => b.text.isNotEmpty);
      EditorFocus.forApp(tv.app).update(
        pageId: garden,
        pageTitle: 'Garden plan',
        blockIds: [block.id],
        text: block.text,
      );
      await showWidget(tester, AgentComposer(store: store, panel: true));
      await tester.tap(key('agent-composer'));
      await tester.enterText(key('agent-composer'), 'Summarize this passage');
      await tapAndSettle(tester, key('agent-send'));
      await settleUntil(
        tester,
        () => store.state?.messages.any((m) => m.role == 'user') ?? false,
      );
      final message = store.state!.messages.firstWhere((m) => m.role == 'user');
      expect(message.text, 'Summarize this passage');
      expect(message.context!.pages.single.id, garden);
      expect(message.context!.selection!.pageId, garden);
      expect(message.context!.selection!.blockIds, [block.id]);
      expect(message.context!.selection!.text, block.text);
    },
  );

  appTest(
    'full composer has mentions only, attaches the picked page and sends human text',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Mentions'));
      EditorFocus.forApp(current.app).update(
        pageId: current.pages['Recipes']!,
        pageTitle: 'Recipes',
        text: 'unused selection',
      );
      await showWidget(tester, AgentComposer(store: store));
      expect(key('agent-context-page'), findsNothing);
      expect(key('agent-context-selection'), findsNothing);
      expectSendAtRight(tester);
      await tester.tap(key('agent-composer'));
      await tester.enterText(key('agent-composer'), '@Garden');
      await settle(tester);
      await tapAndSettle(
        tester,
        key('mention-page-${current.pages['Garden plan']}'),
      );
      expect(
        key('agent-context-mention-${current.pages['Garden plan']}'),
        findsOneWidget,
      );
      expect(
        tester.widget<EditableText>(key('agent-composer')).controller.text,
        '@Garden plan ',
      );
      await tapAndSettle(tester, key('agent-send'));
      await settleUntil(
        tester,
        () => store.state?.messages.any((m) => m.role == 'user') ?? false,
      );
      final message = store.state!.messages.firstWhere((m) => m.role == 'user');
      expect(message.text, '@Garden plan');
      expect(message.context!.pages.single.id, current.pages['Garden plan']);
      expect(message.context!.selection, isNull);
    },
  );

  appTest(
    'untitled pages in chat context and edit labels show their first body line',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final session = (await real(
        tester,
        () => store.createSession(title: 'Chat fixture'),
      ))!;
      await showWidget(
        tester,
        Center(
          child: SizedBox(
            width: 400,
            child: AgentChat(store: store, panel: true),
          ),
        ),
      );
      final untitled = await core(tester, (v) async {
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
      store.debugSetSnapshot(
        snapshot(session: session, untitledPageId: untitled, pageTitle: ''),
      );
      await settleUntil(
        tester,
        () => find.text('Edited Groceries for Sunday').evaluate().isNotEmpty,
      );
      final chip = key('agent-msg-user-page-$untitled');
      expect(
        find.descendant(of: chip, matching: find.text('Groceries for Sunday')),
        findsOneWidget,
      );
      expect(find.text('Edited Untitled'), findsNothing);
      expect(find.text('Untitled'), findsNothing);
    },
  );
}
