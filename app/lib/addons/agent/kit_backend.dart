// Read-only `AgentBackend` for the shared agent kit, over Scratch's agent.
//
// Scratch keeps its own data path (Dart -> flutter_rust_bridge -> NotesAgent).
// This file only projects it onto the kit's port so `AgentChatPane` can draw a
// chat. It reads: `listChats`, `loadChat` and `events`. Every command answers
// with the port's `KitUnsupported`, and the declared capabilities are empty.
//
// Scratch concepts with no kit equivalent are dropped, not approximated; the
// list is in wiki/addons/agent.md.

import 'dart:async';
import 'dart:convert';

import 'package:agent_kit_core/agent_kit_core.dart' as kit;

import '../../src/rust/api/agent.dart';

/// What the backend reads from Scratch's agent. `NotesAgentSource` is the
/// production one; tests pass recorded values through a fake.
abstract interface class KitAgentSource {
  Future<List<AgentSession>> sessions();
  Future<AgentSessionState> sessionState(String sessionId);
  Future<AgentEvents> eventsAfter(int after, String sessionId);

  /// Every session's events; the backend filters by chat.
  Stream<AgentEvent> subscribe();
}

/// [KitAgentSource] over a live `NotesAgent`.
class NotesAgentSource implements KitAgentSource {
  NotesAgentSource(this.agent, {this.events});
  final NotesAgent agent;
  final Stream<AgentEvent>? events;

  @override
  Future<List<AgentSession>> sessions() => agent.sessions();

  @override
  Future<AgentSessionState> sessionState(String sessionId) =>
      agent.sessionState(sessionId: sessionId);

  @override
  Future<AgentEvents> eventsAfter(int after, String sessionId) =>
      agent.eventsAfter(after: after, sessionId: sessionId);

  @override
  Stream<AgentEvent> subscribe() => events ?? agent.subscribe();
}

/// How long burst events (tool starts and ends) wait before one `gap` makes
/// the kit reload the chat.
const kitReloadDelay = Duration(milliseconds: 80);

/// Longest tool output text copied into a row.
const kitToolOutputLimit = 4000;

class ScratchKitBackend implements kit.AgentBackend {
  ScratchKitBackend(this.source, {this.reloadDelay = kitReloadDelay});
  final KitAgentSource source;
  final Duration reloadDelay;
  final _snapshots = <String, AgentSessionState>{};
  final _snapshotListeners = <String, Set<void Function(AgentSessionState)>>{};

  @override
  kit.KitCapabilities get capabilities => kit.KitCapabilities.none();

  @override
  Future<kit.ChatPage> listChats({String? cursor, String? query}) async {
    final needle = query?.trim().toLowerCase() ?? '';
    final sessions = [...await source.sessions()];
    sessions.sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
    return kit.ChatPage(
      chats: [
        for (final s in sessions)
          if (needle.isEmpty || s.title.toLowerCase().contains(needle))
            kitSummary(s),
      ],
    );
  }

  @override
  Future<kit.ChatSnapshot> loadChat(String chatId) async {
    try {
      final state = await source.sessionState(chatId);
      _snapshots[chatId] = state;
      for (final listener in [...?_snapshotListeners[chatId]]) {
        listener(state);
      }
      return kitSnapshot(state);
    } catch (e) {
      throw const kit.KitFailed('That chat could not be loaded.');
    }
  }

  @override
  Stream<kit.KitEvent> events(String chatId, {required int after}) {
    final mapper = KitEventMapper(chatId);
    StreamSubscription<AgentEvent>? live;
    Timer? reload;
    var reloadSeq = 0;
    var lastSeq = after;
    var stopped = false;
    final loaded = _snapshots[chatId];
    late final StreamController<kit.KitEvent> out;

    void seed(AgentSessionState state) {
      mapper.seed(state);
      if (state.seq.toInt() > lastSeq) lastSeq = state.seq.toInt();
    }

    void gap(int seq, {int skipped = 0}) {
      reload?.cancel();
      reload = null;
      out.add(kit.GapEvent(seq: seq, chat: chatId, skipped: skipped));
    }

    void emit(AgentEvent e) {
      if (stopped) return;
      // Lag and reconnect gaps belong to the whole connection, have an empty
      // session, and can carry an old cursor. They still invalidate this chat.
      if (e.kind == 'gap' && (e.sessionId.isEmpty || e.sessionId == chatId)) {
        gap(e.sequence.toInt(), skipped: e.skipped.toInt());
        return;
      }
      if (e.sessionId != chatId) return;
      final seq = e.sequence.toInt();
      if (e.durable) {
        if (seq <= lastSeq) return;
        lastSeq = seq;
      } else if (seq < lastSeq) {
        return;
      }
      final mapped = mapper.map(e);
      for (final event in mapped.events) {
        out.add(event);
      }
      if (mapped.reload) {
        reloadSeq = seq;
        reload ??= Timer(reloadDelay, () {
          reload = null;
          if (!stopped && !out.isClosed) {
            out.add(kit.GapEvent(seq: reloadSeq, chat: chatId));
          }
        });
      }
    }

    Future<void> start() async {
      // Subscribe first, so nothing between the replay and the live stream is
      // lost; duplicates are dropped by sequence.
      final buffered = <AgentEvent>[];
      var replaying = true;
      try {
        live = source.subscribe().listen(
          (e) {
            if (stopped) return;
            replaying ? buffered.add(e) : emit(e);
          },
          onError: (Object error, StackTrace stack) {
            if (!stopped) out.addError(error, stack);
          },
          onDone: () {
            stopped = true;
            reload?.cancel();
            out.close();
          },
        );
        final current = await source.sessionState(chatId);
        if (stopped) return;
        seed(current);
        final past = await source.eventsAfter(after, chatId);
        if (stopped) return;
        if (past.resync) {
          gap(past.lastSeq.toInt());
          lastSeq = past.lastSeq.toInt();
        } else {
          past.events.forEach(emit);
        }
        // The second snapshot may include text produced after loadChat, and
        // live paragraphs buffered during either await may overlap it. A
        // reload reconciles both through Scratch's authoritative snapshot;
        // replaying those paragraphs would append already-seeded text twice.
        final overlaps = buffered.any(
          (e) => e.sessionId == chatId && e.kind == 'paragraph',
        );
        final moved =
            loaded != null &&
            (loaded.seq != current.seq ||
                loaded.liveSeen != current.liveSeen ||
                loaded.streamingText != current.streamingText);
        if (overlaps || moved) gap(lastSeq);
        replaying = false;
        for (final event in buffered) {
          if (overlaps && event.kind == 'paragraph') continue;
          emit(event);
        }
      } catch (e, st) {
        if (!stopped) out.addError(e, st);
        replaying = false;
        if (!stopped) buffered.forEach(emit);
      }
    }

    out = StreamController<kit.KitEvent>(
      onListen: () {
        (_snapshotListeners[chatId] ??= {}).add(seed);
        unawaited(start());
      },
      onCancel: () async {
        stopped = true;
        _snapshotListeners[chatId]?.remove(seed);
        if (_snapshotListeners[chatId]?.isEmpty ?? false) {
          _snapshotListeners.remove(chatId);
        }
        reload?.cancel();
        await live?.cancel();
      },
    );
    return out.stream;
  }

  kit.KitUnsupported _readOnly(kit.KitCapability capability) =>
      kit.KitUnsupported(
        capability,
        'The kit pane is read-only in Scratch for now.',
      );

  @override
  Future<kit.ChatSummary> createChat({String? title, String? workspace}) =>
      Future.error(_readOnly(kit.KitCapability.createChat));

  @override
  Future<kit.SendResult> send(
    String chatId,
    String text, {
    List<kit.KitRef> refs = const [],
  }) => Future.error(_readOnly(kit.KitCapability.send));

  @override
  Future<String> steer(
    String chatId,
    String text, {
    List<kit.KitRef> refs = const [],
  }) => Future.error(_readOnly(kit.KitCapability.steer));

  @override
  Future<String> followup(
    String chatId,
    String text, {
    List<kit.KitRef> refs = const [],
  }) => Future.error(_readOnly(kit.KitCapability.send));

  @override
  Future<void> sendQueuedNow(String chatId, String inputId) =>
      Future.error(_readOnly(kit.KitCapability.sendQueuedNow));

  @override
  Future<void> editQueued(String chatId, String inputId, String text) =>
      Future.error(_readOnly(kit.KitCapability.queueEdit));

  @override
  Future<void> removeQueued(String chatId, String inputId) =>
      Future.error(_readOnly(kit.KitCapability.queueEdit));

  @override
  Future<void> cancel(String chatId) =>
      Future.error(_readOnly(kit.KitCapability.cancel));

  @override
  Future<void> resume(String chatId) =>
      Future.error(_readOnly(kit.KitCapability.resume));

  @override
  Future<void> decideApproval(
    String chatId,
    String approvalId,
    kit.ApprovalDecision decision,
  ) => Future.error(_readOnly(kit.KitCapability.approve));

  @override
  Future<void> answerQuestion(
    String chatId,
    String questionId,
    Map<String, kit.QuestionAnswer> answers,
  ) => Future.error(_readOnly(kit.KitCapability.questions));

  @override
  Future<void> answerChildQuestion(
    String chatId,
    String childId,
    String questionId,
    String text,
  ) => Future.error(_readOnly(kit.KitCapability.answerChildQuestion));

  @override
  Future<void> rename(String chatId, String title) =>
      Future.error(_readOnly(kit.KitCapability.rename));

  @override
  Future<void> archive(String chatId, {bool archived = true}) =>
      Future.error(_readOnly(kit.KitCapability.archive));

  @override
  Future<kit.ChatSummary> fork(String chatId, {String? atItemId}) =>
      Future.error(_readOnly(kit.KitCapability.fork));

  @override
  Future<void> rewind(String chatId, String toItemId) =>
      Future.error(_readOnly(kit.KitCapability.rewind));

  @override
  Future<void> cancelChild(String chatId, String childId) =>
      Future.error(_readOnly(kit.KitCapability.children));

  @override
  Future<void> cancelJob(String chatId, String jobId) =>
      Future.error(_readOnly(kit.KitCapability.jobs));

  @override
  Future<void> undoEdit(String chatId, String editId) =>
      Future.error(_readOnly(kit.KitCapability.editUndo));

  @override
  Future<List<kit.KitChoice>> listModels() =>
      Future.error(_readOnly(kit.KitCapability.models));

  @override
  Future<List<kit.KitChoice>> listProfiles() =>
      Future.error(_readOnly(kit.KitCapability.profiles));

  @override
  Future<void> setModel(String chatId, String modelId) =>
      Future.error(_readOnly(kit.KitCapability.models));

  @override
  Future<void> setProfile(String chatId, String profileId) =>
      Future.error(_readOnly(kit.KitCapability.profiles));

  @override
  Future<kit.ContextDetails> loadContext(String chatId) =>
      Future.error(_readOnly(kit.KitCapability.context));

  @override
  Future<kit.HistoryPage> loadHistoryRange(
    String chatId,
    int firstSeq,
    int lastSeq,
  ) => Future.error(_readOnly(kit.KitCapability.context));

  @override
  Future<kit.ContextSummary> loadSummary(String chatId, String summaryId) =>
      Future.error(_readOnly(kit.KitCapability.context));
}

DateTime? _time(String? s) => s == null ? null : DateTime.tryParse(s);

/// The chat list row for [s]. Scratch's list carries no run state, so every
/// row is idle.
kit.ChatSummary kitSummary(AgentSession s) => kit.ChatSummary(
  id: s.id,
  title: s.title,
  updatedAt: _time(s.updatedAt),
  parentId: s.parentSessionId,
  workspace: s.projectName,
);

kit.RunOutcome? _outcome(String status, String? error) => switch (status) {
  'running' || 'pending' => null,
  final s when s.startsWith('waiting') => null,
  'completed' => kit.RunOutcome.completed,
  'cancelled' => kit.RunOutcome.cancelled,
  'interrupted' => kit.RunOutcome.interrupted,
  'failed' => kit.RunOutcome.failed,
  _ => (error ?? '').isEmpty ? kit.RunOutcome.completed : kit.RunOutcome.failed,
};

kit.RunInfo kitRun(AgentRunRecord r) => kit.RunInfo(
  id: r.id,
  model: r.model.isEmpty ? null : r.model,
  startedAt: _time(r.startedAt),
  endedAt: _time(r.endedAt),
  outcome: _outcome(r.status, r.error),
  error: (r.error ?? '').isEmpty ? null : r.error,
  waitReason: r.status.startsWith('waiting') ? r.status : null,
);

/// The tool's kind from its name; the names are Scratch's vault tools and the
/// runner's file tools.
kit.ToolKind kitToolKind(String name) => switch (name) {
  'search_notes' ||
  'search' ||
  'search_pages' ||
  'search_blocks' ||
  'vault_search' => kit.ToolKind.search,
  'get_page' ||
  'read_page' ||
  'page_get' ||
  'read_file' ||
  'get_blocks' ||
  'query_database' ||
  'agenda' ||
  'read' ||
  'list' => kit.ToolKind.read,
  'edit' => kit.ToolKind.edit,
  'write' ||
  'create_page' ||
  'edit_page' ||
  'upsert_row' ||
  'replace_page' ||
  'update_block' ||
  'delete_block' ||
  'move_blocks' ||
  'move_page' ||
  'trash_page' ||
  'delete_page' ||
  'rename_page' => kit.ToolKind.write,
  'exec' || 'js' => kit.ToolKind.exec,
  _ => kit.ToolKind.other,
};

/// A short target for a tool call: the first of a few well-known argument
/// names that holds a string.
String? kitToolTarget(String argumentsJson) {
  try {
    final args = jsonDecode(argumentsJson);
    if (args is! Map) return null;
    for (final key in const ['path', 'title', 'query', 'page', 'name', 'id']) {
      final v = args[key];
      if (v is String && v.isNotEmpty) return v;
    }
  } on FormatException {
    // Not JSON: no target.
  }
  return null;
}

kit.ApprovalState _approvalState(String s) => switch (s) {
  'approved' => kit.ApprovalState.approved,
  'denied' => kit.ApprovalState.denied,
  'expired' => kit.ApprovalState.expired,
  'cancelled' => kit.ApprovalState.cancelled,
  final s when s.contains('session') => kit.ApprovalState.approvedForSession,
  _ => kit.ApprovalState.pending,
};

kit.Approval kitApproval(AgentApproval a) => kit.Approval(
  id: a.id,
  tool: a.tool,
  requestedAt: _time(a.createdAt) ?? DateTime.fromMillisecondsSinceEpoch(0),
  kind: kitToolKind(a.tool),
  target: kitToolTarget(a.argumentsJson),
  arguments: a.argumentsJson,
  callId: a.callId,
  runId: a.runId,
  state: _approvalState(a.state),
  reason: a.reason,
  decidedAt: _time(a.decidedAt),
);

kit.FileDiff _diff(AgentEdit e) => kit.FileDiff(
  path: e.pageTitle,
  change: switch (e.kind) {
    'created' => kit.FileChange.created,
    'trashed' => kit.FileChange.deleted,
    _ => kit.FileChange.modified,
  },
  hunks: [
    for (final h in e.hunks)
      kit.DiffHunk(
        oldStart: h.oldStart.toInt(),
        newStart: h.newStart.toInt(),
        lines: [
          for (final l in h.lines)
            kit.DiffLine(switch (l.tag) {
              '+' => kit.DiffOp.add,
              '-' => kit.DiffOp.del,
              _ => kit.DiffOp.ctx,
            }, l.text),
        ],
      ),
  ],
);

int _byTime(String a, String b) {
  final x = _time(a), y = _time(b);
  if (x == null || y == null) return a.compareTo(b);
  return x.compareTo(y);
}

/// The chat's snapshot: transcript items in message order, with each
/// assistant turn's tool calls as one activity group and its page edits after
/// it, the pending approvals, the last run and its token counts.
kit.ChatSnapshot kitSnapshot(AgentSessionState s) {
  final runs = [...s.runs]..sort((a, b) => _byTime(a.startedAt, b.startedAt));
  final run = runs.lastOrNull;
  final active = run != null && _outcome(run.status, run.error) == null;
  final results = {
    for (final m in s.messages)
      if (m.role == 'tool' && m.toolCallId != null) m.toolCallId!: m,
  };
  final activity = {for (final a in s.activity) a.callId: a};
  final edits = {for (final e in s.edits) e.id: e};
  final shownEdits = <String>{};

  String? runOf(AgentMessage m) {
    for (final c in m.toolCalls) {
      final id = activity[c.id]?.runId;
      if (id != null) return id;
    }
    return runs
        .where((r) => _byTime(r.startedAt, m.createdAt) <= 0)
        .lastOrNull
        ?.id;
  }

  final items = <kit.TranscriptItem>[];
  for (final m in s.messages) {
    final at = _time(m.createdAt);
    if (m.role == 'user') {
      final ctx = m.context;
      if (m.text.trim().isEmpty && ctx == null) continue;
      items.add(
        kit.UserMessage(
          id: m.id,
          at: at,
          text: m.text,
          refs: [
            for (final p in ctx?.pages ?? const <AgentContextPage>[])
              kit.KitRef(kind: 'page', id: p.id, label: p.title),
            for (final c in ctx?.chats ?? const <AgentContextChat>[])
              kit.KitRef(kind: 'chat', id: c.id, label: c.title),
          ],
        ),
      );
    } else if (m.role == 'assistant') {
      final runId = runOf(m);
      if (m.content.trim().isNotEmpty) {
        items.add(
          kit.AssistantMessage(
            id: m.id,
            at: at,
            markdown: m.content,
            runId: runId,
          ),
        );
      }
      if (m.toolCalls.isEmpty) continue;
      items.add(
        kit.ActivityGroup(
          id: 'act:${m.id}',
          at: at,
          runId: runId,
          rows: [
            for (final c in m.toolCalls)
              _toolRow(
                c,
                results[c.id],
                activity[c.id],
                active && run.id == runId,
              ),
          ],
        ),
      );
      for (final c in m.toolCalls) {
        final edit = edits[activity[c.id]?.editId];
        if (edit == null || !shownEdits.add(edit.id)) continue;
        items.add(
          kit.FileEdit(
            id: 'edit:${edit.id}',
            at: _time(edit.at),
            diff: _diff(edit),
            callId: edit.callId,
            undoable: edit.undoable,
            undone: edit.undone,
          ),
        );
      }
    }
  }
  if (active && s.streamingText.isNotEmpty) {
    items.add(
      kit.AssistantMessage(
        id: kitStreamId(run.id),
        markdown: s.streamingText,
        streaming: true,
        runId: run.id,
      ),
    );
  }

  final tokens = run;
  return kit.ChatSnapshot(
    chatId: s.session.id,
    seq: s.seq.toInt(),
    title: s.session.title,
    parentId: s.session.parentSessionId,
    model: run?.model.isEmpty ?? true ? null : run!.model,
    workspace: switch (s.session.projectName) {
      final name? => kit.WorkspaceInfo(name: name),
      null => null,
    },
    run: run == null ? null : kitRun(run),
    items: items,
    approvals: [for (final a in s.pendingApprovals) kitApproval(a)],
    usage: tokens == null || tokens.inputTokens == null
        ? null
        : kit.Usage(
            inputTokens: tokens.inputTokens?.toInt(),
            outputTokens: tokens.outputTokens?.toInt(),
            cacheReadTokens: tokens.cacheReadTokens?.toInt(),
          ),
  );
}

/// The id of the item that holds a run's streamed text. A reload replaces it
/// with the stored message.
String kitStreamId(String runId) => 'stream:$runId';

kit.ToolRow _toolRow(
  AgentToolCall call,
  AgentMessage? result,
  AgentToolActivity? activity,
  bool runActive,
) {
  final text = result?.content ?? '';
  return kit.ToolRow(
    id: call.id,
    name: call.name,
    kind: kitToolKind(call.name),
    target: kitToolTarget(call.argumentsJson),
    arguments: call.argumentsJson,
    state: result == null
        ? (runActive ? kit.WorkState.running : kit.WorkState.cancelled)
        : (result.isError ? kit.WorkState.failed : kit.WorkState.done),
    meta: (activity?.summary ?? '').isEmpty ? null : activity!.summary,
    output: text.isEmpty
        ? null
        : kit.ToolOutput(
            text: text.length > kitToolOutputLimit
                ? text.substring(0, kitToolOutputLimit)
                : text,
            totalBytes: utf8.encode(text).length,
            truncated: text.length > kitToolOutputLimit,
          ),
  );
}

/// What one Scratch event becomes: kit events to send at once, and whether
/// the chat should be reloaded (a `gap`) because the event changed state the
/// kit's event vocabulary cannot carry (messages, tool rows, queued input).
class KitMapped {
  const KitMapped(this.events, {this.reload = false});
  final List<kit.KitEvent> events;
  final bool reload;
}

/// Maps one chat's Scratch events to kit events. It holds the text offset of
/// each streaming run.
class KitEventMapper {
  KitEventMapper(this.chatId);
  final String chatId;
  final Map<String, String> _text = {};
  final Map<String, int> _textSeq = {};

  /// Starts a run's offset after the text the snapshot already holds.
  void seed(AgentSessionState s) {
    final run = s.runs.where((r) => _outcome(r.status, r.error) == null);
    final active = run.lastOrNull;
    final previous = active == null ? null : _text[active.id];
    final previousSeq = active == null ? null : _textSeq[active.id];
    _text.clear();
    _textSeq.clear();
    if (active != null) {
      // A paragraph may arrive while loadChat's snapshot is in flight. Keep
      // that known tail when the snapshot is an earlier prefix at the same
      // durable position; a newer durable turn can reset the streamed text.
      final keepTail =
          previous != null &&
          previous.startsWith(s.streamingText) &&
          previousSeq != null &&
          previousSeq >= s.seq.toInt();
      final text = keepTail ? previous : s.streamingText;
      // An empty prefix can predate a paragraph received during the load.
      // Keep its known tail, but leave genuinely empty/new turns unseeded so
      // their first paragraph has neither an offset nor a leading separator.
      if (text.isNotEmpty) {
        _text[active.id] = text;
        _textSeq[active.id] = keepTail ? previousSeq : s.seq.toInt();
      }
    }
  }

  KitMapped map(AgentEvent e) {
    final seq = e.sequence.toInt();
    final at = _time(e.createdAt);
    switch (e.kind) {
      case 'run_started':
      case 'run_quota_resumed':
        _text.remove(e.runId);
        _textSeq.remove(e.runId);
        return KitMapped([
          kit.RunStarted(
            seq: seq,
            chat: chatId,
            runId: e.runId,
            model: e.model,
            at: at,
          ),
        ], reload: true);
      case 'run_waiting_for_quota':
        return KitMapped([
          kit.RunWaiting(
            seq: seq,
            chat: chatId,
            runId: e.runId,
            reason: e.reason ?? e.text,
            at: at,
          ),
        ]);
      case 'run_finished':
        _text.remove(e.runId);
        _textSeq.remove(e.runId);
        return KitMapped([
          kit.RunFinished(
            seq: seq,
            chat: chatId,
            runId: e.runId,
            outcome:
                _outcome(e.status ?? '', e.error) ?? kit.RunOutcome.completed,
            error: (e.error ?? '').isEmpty ? null : e.error,
            at: at,
          ),
        ], reload: true);
      case 'paragraph':
        final text = e.text ?? '';
        if (text.isEmpty) return const KitMapped([]);
        final previous = _text[e.runId];
        final start = previous?.length;
        final piece = start == null ? text : '\n\n$text';
        _text[e.runId] = '${previous ?? ''}$piece';
        _textSeq[e.runId] = seq;
        return KitMapped([
          kit.MessageDelta(
            seq: seq,
            chat: chatId,
            itemId: kitStreamId(e.runId),
            offset: start ?? 0,
            text: piece,
            runId: e.runId,
            at: at,
          ),
        ]);
      case 'approval_requested':
        final a = e.approval;
        if (a == null) return const KitMapped([], reload: true);
        return KitMapped([
          kit.ApprovalRequested(
            seq: seq,
            chat: chatId,
            approval: kitApproval(a),
            at: at,
          ),
        ]);
      case 'approval_resolved':
        final id = e.approvalId;
        if (id == null) return const KitMapped([], reload: true);
        return KitMapped([
          kit.ApprovalResolved(
            seq: seq,
            chat: chatId,
            approvalId: id,
            state: _approvalState(e.approvalState ?? ''),
            reason: e.reason,
            at: at,
          ),
        ]);
      case 'gap':
        return KitMapped([
          kit.GapEvent(seq: seq, chat: chatId, skipped: e.skipped.toInt()),
        ]);
      case 'tool_started':
      case 'tool_finished':
      case 'followup_queued':
      case 'steering_accepted':
      case 'session_settled':
        return const KitMapped([], reload: true);
      default:
        // host_state_changed, output, ui_action and the like: nothing to draw.
        return const KitMapped([]);
    }
  }
}
