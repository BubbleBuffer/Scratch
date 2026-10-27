// The kit backend over synthetic Scratch session data (no private content).
import 'dart:async';

import 'package:agent_kit/agent_kit.dart';
import 'package:agent_kit_core/agent_kit_core.dart' as kit;
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/kit_backend.dart';
import 'package:notes_app/src/rust/api/agent.dart';

const _chat = 'chat-1';

AgentSession _session({String id = _chat, String title = 'Plan the week'}) =>
    AgentSession(
      id: id,
      title: title,
      createdAt: '2026-10-20T08:00:00Z',
      updatedAt: '2026-10-20T09:00:00Z',
      projectName: 'Notes',
    );

AgentMessage _msg(
  String id,
  String role,
  String text, {
  String at = '2026-10-20T08:00:01Z',
  List<AgentToolCall> calls = const [],
  String? callId,
  bool isError = false,
  AgentMessageContext? context,
}) => AgentMessage(
  id: id,
  role: role,
  content: text,
  text: text,
  context: context,
  toolCalls: calls,
  toolCallId: callId,
  isError: isError,
  createdAt: at,
);

AgentRunRecord _run(
  String id,
  String status, {
  String started = '2026-10-20T08:00:00Z',
  String? error,
  int? input,
}) => AgentRunRecord(
  id: id,
  sessionId: _chat,
  model: 'fake-model',
  status: status,
  error: error,
  startedAt: started,
  endedAt: status == 'running' ? null : '2026-10-20T08:00:09Z',
  inputTokens: input,
  outputTokens: input == null ? null : 20,
  cacheReadTokens: input == null ? null : 5,
);

AgentSessionState _state({
  List<AgentMessage> messages = const [],
  List<AgentRunRecord> runs = const [],
  List<AgentApproval> approvals = const [],
  List<AgentToolActivity> activity = const [],
  List<AgentEdit> edits = const [],
  int seq = 7,
  String streaming = '',
}) => AgentSessionState(
  session: _session(),
  messages: messages,
  runs: runs,
  pendingApprovals: approvals,
  activity: activity,
  edits: edits,
  seq: seq,
  liveSeen: seq,
  streamingText: streaming,
);

const _readCall = AgentToolCall(
  id: 'call-1',
  name: 'read_page',
  argumentsJson: '{"title":"Groceries"}',
);
const _editCall = AgentToolCall(
  id: 'call-2',
  name: 'edit_page',
  argumentsJson: '{"title":"Groceries","text":"milk"}',
);

AgentEdit _edit() => const AgentEdit(
  id: 'edit-1',
  sessionId: _chat,
  runId: 'run-1',
  callId: 'call-2',
  at: '2026-10-20T08:00:05Z',
  pageId: 'page-1',
  pageTitle: 'Groceries',
  kind: 'edited',
  added: 1,
  removed: 1,
  hunks: [
    AgentDiffHunk(
      oldStart: 1,
      newStart: 1,
      lines: [
        AgentDiffLine(tag: '=', text: 'Groceries'),
        AgentDiffLine(tag: '-', text: 'eggs'),
        AgentDiffLine(tag: '+', text: 'milk'),
      ],
    ),
  ],
  changedBlocks: ['b1'],
  undone: false,
  undoable: true,
);

AgentEvent _event(
  String kind, {
  int seq = 8,
  bool durable = true,
  String session = _chat,
  String run = 'run-1',
  String? text,
  String? status,
  String? error,
  AgentApproval? approval,
  String? approvalId,
  String? approvalState,
}) => AgentEvent(
  kind: kind,
  sequence: seq,
  durable: durable,
  sessionId: session,
  runId: run,
  createdAt: '2026-10-20T08:00:06Z',
  text: text,
  status: status,
  error: error,
  approval: approval,
  approvalId: approvalId,
  approvalState: approvalState,
  isError: false,
  skipped: 0,
  json: '{}',
);

class _Source implements KitAgentSource {
  _Source({this.sessionList = const [], AgentSessionState? state, this.past})
    : state = state ?? _state();
  List<AgentSession> sessionList;
  AgentSessionState state;
  AgentEvents? past;
  final live = StreamController<AgentEvent>.broadcast();
  int afterAsked = -1;

  @override
  Future<List<AgentSession>> sessions() async => sessionList;
  @override
  Future<AgentSessionState> sessionState(String sessionId) async => state;
  @override
  Future<AgentEvents> eventsAfter(int after, String sessionId) async {
    afterAsked = after;
    return past ?? AgentEvents(events: const [], lastSeq: after, resync: false);
  }

  @override
  Stream<AgentEvent> subscribe() => live.stream;
}

Future<void> _settle() =>
    Future<void>.delayed(const Duration(milliseconds: 20));

void main() {
  pumpPaneTests();
  group('snapshot', () {
    test('maps messages, tool rows, edits, approvals, run and usage', () {
      final snap = kitSnapshot(
        _state(
          messages: [
            _msg(
              'm1',
              'user',
              'Add milk to groceries',
              context: const AgentMessageContext(
                pages: [AgentContextPage(id: 'page-1', title: 'Groceries')],
              ),
            ),
            _msg(
              'm2',
              'assistant',
              'Let me look.',
              at: '2026-10-20T08:00:02Z',
              calls: [_readCall, _editCall],
            ),
            _msg(
              'm3',
              'tool',
              'Groceries: eggs',
              at: '2026-10-20T08:00:03Z',
              callId: 'call-1',
            ),
            _msg(
              'm4',
              'tool',
              'denied',
              at: '2026-10-20T08:00:04Z',
              callId: 'call-2',
              isError: true,
            ),
            _msg('m5', 'assistant', 'Done.', at: '2026-10-20T08:00:08Z'),
          ],
          runs: [_run('run-1', 'completed', input: 100)],
          approvals: [
            const AgentApproval(
              id: 'ap-1',
              sessionId: _chat,
              runId: 'run-1',
              callId: 'call-2',
              tool: 'edit_page',
              argumentsJson: '{"title":"Groceries"}',
              state: 'pending',
              createdAt: '2026-10-20T08:00:04Z',
            ),
          ],
          activity: [
            const AgentToolActivity(
              runId: 'run-1',
              callId: 'call-1',
              order: 0,
              name: 'read_page',
              summary: 'Read Groceries',
              isError: false,
            ),
            const AgentToolActivity(
              runId: 'run-1',
              callId: 'call-2',
              order: 1,
              name: 'edit_page',
              summary: 'Edited Groceries',
              isError: false,
              editId: 'edit-1',
            ),
          ],
          edits: [_edit()],
        ),
      );

      expect(snap.chatId, _chat);
      expect(snap.seq, 7);
      expect(snap.title, 'Plan the week');
      expect(snap.workspace?.name, 'Notes');
      expect(snap.model, 'fake-model');
      expect(snap.run?.outcome, kit.RunOutcome.completed);
      expect(snap.usage?.inputTokens, 100);
      expect(snap.usage?.cacheReadTokens, 5);

      expect(snap.items.map((i) => i.runtimeType.toString()).toList(), [
        'UserMessage',
        'AssistantMessage',
        'ActivityGroup',
        'FileEdit',
        'AssistantMessage',
      ]);
      final user = snap.items[0] as kit.UserMessage;
      expect(user.text, 'Add milk to groceries');
      expect(user.refs.single.kind, 'page');
      expect(user.refs.single.label, 'Groceries');

      final rows = (snap.items[2] as kit.ActivityGroup).rows
          .cast<kit.ToolRow>();
      expect(rows.map((r) => r.id), ['call-1', 'call-2']);
      expect(rows[0].kind, kit.ToolKind.read);
      expect(rows[0].target, 'Groceries');
      expect(rows[0].state, kit.WorkState.done);
      expect(rows[0].meta, 'Read Groceries');
      expect(rows[0].output?.text, 'Groceries: eggs');
      expect(rows[1].kind, kit.ToolKind.write);
      expect(rows[1].state, kit.WorkState.failed);

      final edit = snap.items[3] as kit.FileEdit;
      expect(edit.undoable, isTrue);
      expect(edit.diff.path, 'Groceries');
      expect(edit.diff.added, 1);
      expect(edit.diff.removed, 1);
      expect(edit.diff.hunks.single.lines.first.op, kit.DiffOp.ctx);

      expect(snap.approvals.single.isPending, isTrue);
      expect(snap.approvals.single.tool, 'edit_page');
      expect(snap.status, kit.ChatStatus.needsYou);
    });

    test('an active run keeps its streamed text as a streaming message', () {
      final snap = kitSnapshot(
        _state(
          messages: [_msg('m1', 'user', 'Hi')],
          runs: [_run('run-1', 'running')],
          streaming: 'Hello\n\nthere',
        ),
      );
      final last = snap.items.last as kit.AssistantMessage;
      expect(last.streaming, isTrue);
      expect(last.id, kitStreamId('run-1'));
      expect(last.markdown, 'Hello\n\nthere');
      expect(snap.status, kit.ChatStatus.running);
    });

    test('an unanswered tool call of a finished run is cancelled', () {
      final snap = kitSnapshot(
        _state(
          messages: [
            _msg('m1', 'assistant', '', calls: [_readCall]),
          ],
          runs: [_run('run-1', 'cancelled')],
        ),
      );
      final row =
          (snap.items.single as kit.ActivityGroup).rows.single as kit.ToolRow;
      expect(row.state, kit.WorkState.cancelled);
      expect(snap.run?.outcome, kit.RunOutcome.cancelled);
    });

    test('run statuses become outcomes', () {
      kit.RunOutcome? of(String status, [String? error]) =>
          kitRun(_run('r', status, error: error)).outcome;
      expect(of('running'), isNull);
      expect(of('pending'), isNull);
      expect(of('waiting_for_quota'), isNull);
      expect(of('completed'), kit.RunOutcome.completed);
      expect(of('failed', 'boom'), kit.RunOutcome.failed);
      expect(of('interrupted'), kit.RunOutcome.interrupted);
      expect(of('weird', 'boom'), kit.RunOutcome.failed);
    });

    test('the last run decides the status, newest by start time', () {
      final snap = kitSnapshot(
        _state(
          runs: [
            _run(
              'run-2',
              'failed',
              started: '2026-10-20T09:00:00Z',
              error: 'x',
            ),
            _run('run-1', 'completed'),
          ],
        ),
      );
      expect(snap.run?.id, 'run-2');
      expect(snap.status, kit.ChatStatus.failed);
      expect(snap.run?.error, 'x');
    });

    test('long tool output is cut and flagged', () {
      final long = 'x' * (kitToolOutputLimit + 10);
      final snap = kitSnapshot(
        _state(
          messages: [
            _msg('m1', 'assistant', '', calls: [_readCall]),
            _msg('m2', 'tool', long, callId: 'call-1'),
          ],
          runs: [_run('run-1', 'completed')],
        ),
      );
      final row =
          (snap.items.single as kit.ActivityGroup).rows.single as kit.ToolRow;
      expect(row.output?.text.length, kitToolOutputLimit);
      expect(row.output?.truncated, isTrue);
      expect(row.output?.totalBytes, long.length);
    });

    test('tool kinds and targets', () {
      expect(kitToolKind('search_notes'), kit.ToolKind.search);
      expect(kitToolKind('exec'), kit.ToolKind.exec);
      expect(kitToolKind('mystery'), kit.ToolKind.other);
      expect(kitToolTarget('{"query":"milk"}'), 'milk');
      expect(kitToolTarget('not json'), isNull);
      expect(kitToolTarget('[1]'), isNull);
    });

    test('the snapshot survives the kit JSON round trip', () {
      final snap = kitSnapshot(
        _state(
          messages: [_msg('m1', 'user', 'Hi')],
          runs: [_run('run-1', 'completed')],
        ),
      );
      final again = kit.ChatSnapshot.fromJson(snap.toJson());
      expect(again.items.length, 1);
      expect(again.seq, snap.seq);
    });
  });

  group('backend', () {
    test('declares no optional capability', () {
      final backend = ScratchKitBackend(_Source());
      expect(backend.capabilities.values, isEmpty);
    });

    test('lists chats newest first and filters by title', () async {
      final backend = ScratchKitBackend(
        _Source(
          sessionList: [
            AgentSession(
              id: 'old',
              title: 'Budget',
              createdAt: '2026-10-01T08:00:00Z',
              updatedAt: '2026-10-01T08:00:00Z',
            ),
            _session(id: 'new', title: 'Trip plan'),
          ],
        ),
      );
      final all = await backend.listChats();
      expect(all.chats.map((c) => c.id), ['new', 'old']);
      expect(all.chats.first.workspace, 'Notes');
      expect(all.nextCursor, isNull);
      final some = await backend.listChats(query: 'bud');
      expect(some.chats.map((c) => c.id), ['old']);
    });

    test('commands answer unsupported', () async {
      final backend = ScratchKitBackend(_Source());
      final calls = <Future<Object?> Function()>[
        () => backend.send(_chat, 'hi'),
        () => backend.steer(_chat, 'hi'),
        () => backend.followup(_chat, 'hi'),
        () => backend.createChat(),
        () => backend.cancel(_chat),
        () => backend.decideApproval(_chat, 'ap', kit.ApprovalDecision.approve),
        () => backend.undoEdit(_chat, 'e'),
        () => backend.rename(_chat, 'x'),
        () => backend.archive(_chat),
        () => backend.listModels(),
        () => backend.loadContext(_chat),
      ];
      for (final call in calls) {
        await expectLater(call(), throwsA(isA<kit.KitUnsupported>()));
      }
    });

    test('a failed load is a user-safe error', () async {
      final source = _FailingSource();
      await expectLater(
        ScratchKitBackend(source).loadChat(_chat),
        throwsA(
          isA<kit.KitFailed>().having(
            (e) => e.message,
            'message',
            isNot(contains('secret')),
          ),
        ),
      );
    });
  });

  group('events', () {
    test('replays durable events after the snapshot, then goes live', () async {
      final source = _Source(
        state: _state(runs: [_run('run-1', 'running')]),
        past: AgentEvents(
          events: [
            _event('run_started', seq: 8),
            _event(
              'approval_requested',
              seq: 9,
              approval: const AgentApproval(
                id: 'ap-1',
                sessionId: _chat,
                runId: 'run-1',
                callId: 'call-2',
                tool: 'edit_page',
                argumentsJson: '{}',
                state: 'pending',
                createdAt: '2026-10-20T08:00:04Z',
              ),
            ),
          ],
          lastSeq: 9,
          resync: false,
        ),
      );
      final backend = ScratchKitBackend(
        source,
        reloadDelay: const Duration(milliseconds: 5),
      );
      final got = <kit.KitEvent>[];
      final sub = backend.events(_chat, after: 7).listen(got.add);
      await _settle();
      expect(source.afterAsked, 7);
      expect(got.map((e) => e.kind), [
        'run.started',
        'approval.requested',
        'gap',
      ]);

      source.live.add(
        _event(
          'approval_resolved',
          seq: 10,
          approvalId: 'ap-1',
          approvalState: 'approved_session',
        ),
      );
      source.live.add(_event('run_finished', seq: 11, status: 'completed'));
      await _settle();
      final resolved = got.whereType<kit.ApprovalResolved>().single;
      expect(resolved.state, kit.ApprovalState.approvedForSession);
      final finished = got.whereType<kit.RunFinished>().single;
      expect(finished.outcome, kit.RunOutcome.completed);
      expect(got.last, isA<kit.GapEvent>());
      await sub.cancel();
    });

    test('drops other chats, old sequences and duplicates', () async {
      final source = _Source();
      final backend = ScratchKitBackend(source);
      final got = <kit.KitEvent>[];
      final sub = backend.events(_chat, after: 7).listen(got.add);
      await _settle();
      source.live
        ..add(_event('run_started', seq: 5))
        ..add(_event('run_started', seq: 9, session: 'other'))
        ..add(_event('run_started', seq: 9))
        ..add(_event('run_started', seq: 9));
      await _settle();
      expect(got.whereType<kit.RunStarted>(), hasLength(1));
      await sub.cancel();
    });

    test('a burst of tool events makes one gap', () async {
      final source = _Source();
      final backend = ScratchKitBackend(
        source,
        reloadDelay: const Duration(milliseconds: 15),
      );
      final got = <kit.KitEvent>[];
      final sub = backend.events(_chat, after: 7).listen(got.add);
      await _settle();
      source.live
        ..add(_event('tool_started', seq: 8))
        ..add(_event('tool_finished', seq: 9))
        ..add(_event('session_settled', seq: 10));
      await Future<void>.delayed(const Duration(milliseconds: 60));
      final gaps = got.whereType<kit.GapEvent>().toList();
      expect(gaps, hasLength(1));
      expect(gaps.single.seq, 10);
      await sub.cancel();
    });

    test('a resync from the store becomes a gap', () async {
      final source = _Source(
        past: AgentEvents(events: const [], lastSeq: 40, resync: true),
      );
      final got = <kit.KitEvent>[];
      final sub = ScratchKitBackend(source)
          .events(_chat, after: 3)
          .listen(got.add);
      await _settle();
      expect(got.single, isA<kit.GapEvent>());
      expect(got.single.seq, 40);
      await sub.cancel();
    });

    test(
      'paragraphs stream at increasing offsets, after the snapshot text',
      () async {
        final source = _Source(
          state: _state(runs: [_run('run-1', 'running')], streaming: 'First'),
        );
        final backend = ScratchKitBackend(source);
        final got = <kit.KitEvent>[];
        final sub = backend.events(_chat, after: 7).listen(got.add);
        await _settle();
        source.live
          ..add(_event('paragraph', seq: 7, durable: false, text: 'Second'))
          ..add(_event('paragraph', seq: 7, durable: false, text: 'Third'));
        await _settle();
        final deltas = got.whereType<kit.MessageDelta>().toList();
        expect(deltas.map((d) => d.offset), [5, 5 + 2 + 6]);
        expect(deltas.map((d) => d.text), ['\n\nSecond', '\n\nThird']);
        expect(deltas.first.itemId, kitStreamId('run-1'));

        // Folding them into the snapshot reads as one message.
        var snap = kitSnapshot(source.state);
        for (final d in deltas) {
          snap = kit.reduce(snap, d);
        }
        expect(
          (snap.items.last as kit.AssistantMessage).markdown,
          'First\n\nSecond\n\nThird',
        );
        await sub.cancel();
      },
    );

    test('without a snapshot text the first paragraph starts at zero', () {
      final mapper = KitEventMapper(_chat);
      final first = mapper.map(
        _event('paragraph', seq: 8, durable: false, text: 'One'),
      );
      final second = mapper.map(
        _event('paragraph', seq: 8, durable: false, text: 'Two'),
      );
      final a = first.events.single as kit.MessageDelta;
      final b = second.events.single as kit.MessageDelta;
      expect([a.offset, a.text], [0, 'One']);
      expect([b.offset, b.text], [3, '\n\nTwo']);
    });

    test('unknown event kinds draw nothing', () {
      final mapped = KitEventMapper(_chat).map(_event('host_state_changed'));
      expect(mapped.events, isEmpty);
      expect(mapped.reload, isFalse);
    });
  });
}

class _FailingSource extends _Source {
  @override
  Future<AgentSessionState> sessionState(String sessionId) async =>
      throw StateError('secret path /home/user/vault');
}

void pumpPaneTests() {
  group('pane', () {
    testWidgets('AgentChatPane draws a recorded chat from the backend', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final source = _Source(
        state: _state(
          messages: [
            _msg('m1', 'user', 'Add milk to groceries'),
            _msg('m2', 'assistant', 'Done, milk is on the list.'),
          ],
          runs: [_run('run-1', 'completed')],
        ),
      );
      await tester.pumpWidget(
        Directionality(
          textDirection: TextDirection.ltr,
          child: MediaQuery(
            data: const MediaQueryData(size: Size(1200, 900)),
            child: BbThemeScope(
              brightness: Brightness.light,
              child: AgentChatPane(
                backend: ScratchKitBackend(source),
                chatId: _chat,
              ),
            ),
          ),
        ),
      );
      await tester.pump(const Duration(milliseconds: 50));
      await tester.pump(const Duration(milliseconds: 50));
      expect(find.text('Add milk to groceries'), findsOneWidget);
      expect(find.textContaining('milk is on the list'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    });
  });
}
