// The kit backend over synthetic Scratch session data (no private content).
import 'dart:async';

import 'package:agent_kit/agent_kit.dart';
import 'package:agent_kit_core/agent_kit_core.dart' as kit;
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/agent_event_feed.dart';
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
  int liveSeen = 0,
  String streaming = '',
}) => AgentSessionState(
  session: _session(),
  messages: messages,
  runs: runs,
  pendingApprovals: approvals,
  activity: activity,
  edits: edits,
  seq: seq,
  liveSeen: liveSeen,
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
  int skipped = 0,
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
  skipped: skipped,
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

class _ControlledSource extends _Source {
  Completer<AgentSessionState>? snapshot;
  Completer<AgentEvents>? replay;

  @override
  Future<AgentSessionState> sessionState(String sessionId) =>
      snapshot?.future ?? super.sessionState(sessionId);

  @override
  Future<AgentEvents> eventsAfter(int after, String sessionId) {
    afterAsked = after;
    return replay?.future ?? super.eventsAfter(after, sessionId);
  }
}

/// Models the bridge's connection-wide close, rather than closing only the
/// particular Dart listener that the kit happens to own.
class _Connection implements NotesAgent {
  final streams = <StreamController<AgentEvent>>[];
  AgentSessionState state = _state();

  @override
  Stream<AgentEvent> subscribe({String? sessionId}) {
    final stream = StreamController<AgentEvent>.broadcast();
    streams.add(stream);
    return stream.stream;
  }

  void emit(AgentEvent event) {
    for (final stream in streams) {
      if (!stream.isClosed) stream.add(event);
    }
  }

  @override
  Future<void> closeEvents() async {
    await Future.wait([for (final stream in streams) stream.close()]);
  }

  @override
  dynamic noSuchMethod(Invocation invocation) =>
      switch (invocation.memberName) {
        #sessionState => Future.value(state),
        #eventsAfter => Future.value(
          AgentEvents(events: const [], lastSeq: state.seq, resync: false),
        ),
        _ => super.noSuchMethod(invocation),
      };
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
    test('declares no capabilities, including writes', () {
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
      for (final entry in <kit.KitCapability, Future<Object?> Function()>{
        kit.KitCapability.createChat: () => backend.createChat(),
        kit.KitCapability.send: () => backend.send(_chat, 'hi'),
        kit.KitCapability.cancel: () => backend.cancel(_chat),
        kit.KitCapability.approve: () =>
            backend.decideApproval(_chat, 'ap', kit.ApprovalDecision.approve),
      }.entries) {
        await expectLater(
          entry.value(),
          throwsA(
            isA<kit.KitUnsupported>().having(
              (e) => e.capability,
              'capability',
              entry.key,
            ),
          ),
        );
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
    test(
      'store feed can be read before the first subscription binds',
      () async {
        final connection = _Connection();
        final feed = AgentEventFeed()..open(connection);
        final got = <AgentEvent>[];
        var done = false;
        final reader = feed
            .eventsFor(connection)
            .listen(got.add, onDone: () => done = true);
        await _settle();
        expect(done, isFalse);
        final sub = feed.bind(
          connection,
          connection.subscribe(),
          onEvent: (_) {},
          onError: (_) {},
        );
        connection.emit(_event('run_started'));
        await _settle();
        expect(got.single.kind, 'run_started');
        feed.close(connection);
        await reader.cancel();
        await sub.cancel();
        await connection.closeEvents();
      },
    );

    test(
      'old live paragraphs do not change the current streaming offset',
      () async {
        final source = _Source(
          state: _state(
            runs: [_run('run-1', 'running')],
            streaming: 'First',
            liveSeen: 1,
          ),
        );
        final got = <kit.KitEvent>[];
        final sub = ScratchKitBackend(source)
            .events(_chat, after: 7)
            .listen(got.add);
        await _settle();
        source.live
          ..add(_event('paragraph', seq: 6, durable: false, text: 'Old'))
          ..add(_event('paragraph', seq: 7, durable: false, text: 'Next'));
        await _settle();
        final delta = got.whereType<kit.MessageDelta>().single;
        expect(delta.offset, 5);
        expect(delta.text, '\n\nNext');
        await sub.cancel();
        await source.live.close();
      },
    );

    test(
      'store feed survives closeEvents and reports its replacement gap',
      () async {
        final connection = _Connection();
        final feed = AgentEventFeed();
        var storeEvents = 0;
        var storeSub = feed.bind(
          connection,
          connection.subscribe(),
          onEvent: (_) => storeEvents++,
          onError: (_) {},
        );
        final backend = ScratchKitBackend(
          NotesAgentSource(connection, events: feed.eventsFor(connection)),
        );
        final got = <kit.KitEvent>[];
        var done = false;
        final sub = backend
            .events(_chat, after: 7)
            .listen(got.add, onDone: () => done = true);
        await _settle();
        connection.emit(
          _event('paragraph', seq: 7, durable: false, text: 'One'),
        );
        await _settle();
        expect(got.whereType<kit.MessageDelta>(), hasLength(1));

        final cancelled = storeSub.cancel();
        await connection.closeEvents();
        await cancelled;
        expect(done, isFalse);
        storeSub = feed.bind(
          connection,
          connection.subscribe(),
          onEvent: (_) => storeEvents++,
          onError: (_) {},
        );
        await _settle();
        expect(got.whereType<kit.GapEvent>(), hasLength(1));
        connection.emit(
          _event(
            'approval_resolved',
            seq: 8,
            approvalId: 'ap',
            approvalState: 'denied',
          ),
        );
        await _settle();
        expect(got.whereType<kit.ApprovalResolved>(), hasLength(1));
        expect(storeEvents, 2);
        expect(done, isFalse);
        feed.close(connection);
        await _settle();
        expect(done, isTrue);
        await sub.cancel();
        await storeSub.cancel();
        await connection.closeEvents();
      },
    );

    test('a closed store feed never follows a newer connection', () async {
      final old = _Connection(), next = _Connection();
      final feed = AgentEventFeed();
      final oldSub = feed.bind(
        old,
        old.subscribe(),
        onEvent: (_) {},
        onError: (_) {},
      );
      final events = <AgentEvent>[];
      var done = false;
      final reader = feed
          .eventsFor(old)
          .listen(events.add, onDone: () => done = true);
      final newSub = feed.bind(
        next,
        next.subscribe(),
        onEvent: (_) {},
        onError: (_) {},
      );
      old.emit(_event('run_started'));
      next.emit(_event('run_finished'));
      await _settle();
      expect(done, isTrue);
      expect(events, isEmpty);
      expect(await feed.eventsFor(old).toList(), isEmpty);
      // A late close for the previous connection must not close the new one.
      feed.close(old);
      final newest = <AgentEvent>[];
      final newReader = feed.eventsFor(next).listen(newest.add);
      next.emit(_event('run_started', seq: 10));
      await _settle();
      expect(newest.single.sequence, 10);
      feed.close(next);
      await reader.cancel();
      await newReader.cancel();
      await oldSub.cancel();
      await newSub.cancel();
      await old.closeEvents();
      await next.closeEvents();
    });

    test(
      'buffered paragraphs overlapping startup reload without duplication',
      () async {
        final source = _ControlledSource();
        final pending = source.snapshot = Completer<AgentSessionState>();
        final backend = ScratchKitBackend(source);
        final got = <kit.KitEvent>[];
        final sub = backend.events(_chat, after: 7).listen(got.add);
        source.live.add(
          _event('paragraph', seq: 7, durable: false, text: 'First'),
        );
        await _settle();
        source.state = _state(
          runs: [_run('run-1', 'running')],
          streaming: 'First',
          liveSeen: 1,
        );
        pending.complete(source.state);
        await _settle();
        expect(got.whereType<kit.MessageDelta>(), isEmpty);
        expect(got.whereType<kit.GapEvent>(), hasLength(1));
        source.snapshot = null;
        var snapshot = await backend.loadChat(_chat);
        source.live.add(
          _event('paragraph', seq: 7, durable: false, text: 'Second'),
        );
        await _settle();
        snapshot = kit.reduce(
          snapshot,
          got.whereType<kit.MessageDelta>().single,
        );
        expect(
          (snapshot.items.single as kit.AssistantMessage).markdown,
          'First\n\nSecond',
        );
        await sub.cancel();
        await source.live.close();
      },
    );

    test(
      'a second snapshot ahead of loadChat reconciles through a gap',
      () async {
        final source = _Source(
          state: _state(
            runs: [_run('run-1', 'running')],
            streaming: 'First',
            liveSeen: 1,
          ),
        );
        final backend = ScratchKitBackend(source);
        await backend.loadChat(_chat);
        source.state = _state(
          runs: [_run('run-1', 'running')],
          streaming: 'First\n\nSecond',
          liveSeen: 2,
        );
        final got = <kit.KitEvent>[];
        final sub = backend.events(_chat, after: 7).listen(got.add);
        await _settle();
        expect(got.whereType<kit.GapEvent>(), hasLength(1));
        var snapshot = await backend.loadChat(_chat);
        source.live.add(
          _event('paragraph', seq: 7, durable: false, text: 'Third'),
        );
        await _settle();
        snapshot = kit.reduce(
          snapshot,
          got.whereType<kit.MessageDelta>().single,
        );
        expect(
          (snapshot.items.single as kit.AssistantMessage).markdown,
          'First\n\nSecond\n\nThird',
        );
        await sub.cancel();
        await source.live.close();
      },
    );

    test('a gap reload rebases the active streaming offset', () async {
      final source = _Source(
        state: _state(
          runs: [_run('run-1', 'running')],
          streaming: 'First',
          liveSeen: 1,
        ),
      );
      final backend = ScratchKitBackend(source);
      await backend.loadChat(_chat);
      final got = <kit.KitEvent>[];
      final sub = backend.events(_chat, after: 7).listen(got.add);
      await _settle();
      source.state = _state(
        runs: [_run('run-1', 'running')],
        streaming: 'First\n\nLost',
        liveSeen: 2,
      );
      source.live.add(_event('gap', seq: 7, durable: false, session: ''));
      await _settle();
      var snapshot = await backend.loadChat(_chat);
      source.live.add(
        _event('paragraph', seq: 7, durable: false, text: 'Next'),
      );
      await _settle();
      final delta = got.whereType<kit.MessageDelta>().single;
      expect(delta.offset, 'First\n\nLost'.length);
      snapshot = kit.reduce(snapshot, delta);
      expect(
        (snapshot.items.single as kit.AssistantMessage).markdown,
        'First\n\nLost\n\nNext',
      );
      await sub.cancel();
      await source.live.close();
    });

    test(
      'a paragraph arriving during reload keeps the following offset',
      () async {
        final source = _ControlledSource();
        source.state = _state(
          runs: [_run('run-1', 'running')],
          streaming: 'First',
          liveSeen: 1,
        );
        final backend = ScratchKitBackend(source);
        await backend.loadChat(_chat);
        final got = <kit.KitEvent>[];
        final sub = backend.events(_chat, after: 7).listen(got.add);
        await _settle();
        source.live.add(
          _event('paragraph', seq: 7, durable: false, text: 'Second'),
        );
        await _settle();
        final pending = source.snapshot = Completer<AgentSessionState>();
        final loading = backend.loadChat(_chat);
        source.live.add(
          _event('paragraph', seq: 7, durable: false, text: 'Third'),
        );
        await _settle();
        pending.complete(
          _state(
            runs: [_run('run-1', 'running')],
            streaming: 'First\n\nSecond',
            liveSeen: 2,
          ),
        );
        var snapshot = await loading;
        snapshot = kit.reduce(snapshot, got.whereType<kit.MessageDelta>().last);
        source.live.add(
          _event('paragraph', seq: 7, durable: false, text: 'Fourth'),
        );
        await _settle();
        final delta = got.whereType<kit.MessageDelta>().last;
        expect(delta.offset, 'First\n\nSecond\n\nThird'.length);
        snapshot = kit.reduce(snapshot, delta);
        expect(
          (snapshot.items.single as kit.AssistantMessage).markdown,
          'First\n\nSecond\n\nThird\n\nFourth',
        );
        await sub.cancel();
        await source.live.close();
      },
    );

    test(
      'an empty reload preserves a paragraph that arrived in flight',
      () async {
        final source = _ControlledSource();
        source.state = _state(runs: [_run('run-1', 'running')]);
        final backend = ScratchKitBackend(source);
        await backend.loadChat(_chat);
        final got = <kit.KitEvent>[];
        final sub = backend.events(_chat, after: 7).listen(got.add);
        await _settle();
        final pending = source.snapshot = Completer<AgentSessionState>();
        final loading = backend.loadChat(_chat);
        source.live.add(
          _event('paragraph', seq: 7, durable: false, text: 'First'),
        );
        await _settle();
        pending.complete(_state(runs: [_run('run-1', 'running')]));
        var snapshot = await loading;
        snapshot = kit.reduce(
          snapshot,
          got.whereType<kit.MessageDelta>().single,
        );
        source.live.add(
          _event('paragraph', seq: 7, durable: false, text: 'Second'),
        );
        await _settle();
        final delta = got.whereType<kit.MessageDelta>().last;
        expect(delta.offset, 'First'.length);
        expect(delta.text, '\n\nSecond');
        snapshot = kit.reduce(snapshot, delta);
        expect(
          (snapshot.items.single as kit.AssistantMessage).markdown,
          'First\n\nSecond',
        );
        await sub.cancel();
        await source.live.close();
      },
    );

    for (final run in ['run-1', 'run-2']) {
      test('a newer durable empty snapshot resets text for $run', () {
        final mapper = KitEventMapper(_chat);
        mapper.seed(_state(runs: [_run('run-1', 'running')]));
        mapper.map(_event('paragraph', seq: 7, durable: false, text: 'Old'));
        // A newer durable phase may clear streaming text even within the
        // same run. A different active run must also begin at zero.
        mapper.seed(_state(seq: 8, runs: [_run(run, 'running')]));
        final delta =
            mapper
                    .map(
                      _event(
                        'paragraph',
                        seq: 8,
                        durable: false,
                        run: run,
                        text: 'New',
                      ),
                    )
                    .events
                    .single
                as kit.MessageDelta;
        expect(delta.offset, 0);
        expect(delta.text, 'New');
      });
    }

    for (final fail in [false, true]) {
      test(
        'cancelling pending startup snapshot ignores late ${fail ? 'failure' : 'completion'}',
        () async {
          final source = _ControlledSource();
          final pending = source.snapshot = Completer<AgentSessionState>();
          final got = <kit.KitEvent>[];
          final errors = <Object>[];
          final sub = ScratchKitBackend(source)
              .events(_chat, after: 7)
              .listen(got.add, onError: errors.add);
          await sub.cancel();
          if (fail) {
            pending.completeError(StateError('late startup failure'));
          } else {
            pending.complete(_state());
          }
          await _settle();
          expect(source.afterAsked, -1);
          expect(got, isEmpty);
          expect(errors, isEmpty);
          expect(source.live.hasListener, isFalse);
          await source.live.close();
        },
      );
    }

    test(
      'cancelling pending replay drops replay, buffered events and timers',
      () async {
        final source = _ControlledSource();
        final pending = source.replay = Completer<AgentEvents>();
        final got = <kit.KitEvent>[];
        final errors = <Object>[];
        final sub = ScratchKitBackend(
          source,
          reloadDelay: Duration.zero,
        ).events(_chat, after: 7).listen(got.add, onError: errors.add);
        await _settle();
        expect(source.afterAsked, 7);
        source.live.add(_event('tool_finished', seq: 9));
        await _settle();
        await sub.cancel();
        pending.complete(
          AgentEvents(
            events: [_event('run_started')],
            lastSeq: 9,
            resync: true,
          ),
        );
        await _settle();
        expect(got, isEmpty);
        expect(errors, isEmpty);
        expect(source.live.hasListener, isFalse);
        await source.live.close();
      },
    );

    test('a connection-wide live gap reloads the selected chat', () async {
      final source = _Source();
      final got = <kit.KitEvent>[];
      final sub = ScratchKitBackend(source)
          .events(_chat, after: 7)
          .listen(got.add);
      await _settle();
      // Real lag/reconnect notifications have no session and are live, not
      // durable. Their cursor may equal the snapshot's durable sequence.
      source.live.add(
        _event('gap', session: '', seq: 7, durable: false, skipped: 4),
      );
      await _settle();
      final gaps = got.whereType<kit.GapEvent>().toList();
      expect(gaps, hasLength(1));
      expect(gaps.single.chat, _chat);
      expect(gaps.single.seq, 7);
      expect(gaps.single.skipped, 4);
      await sub.cancel();
      await source.live.close();
    });

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
      expect(find.text('This chat is read-only.'), findsOneWidget);
      expect(
        tester.widget<EditableText>(find.byType(EditableText).last).readOnly,
        isTrue,
      );
      await tester.pumpWidget(const SizedBox());
    });
  });
}
