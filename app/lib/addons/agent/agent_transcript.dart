// What the chat shows, built from a session snapshot: user and assistant
// text, compact assistant turns with edits and work, quiet lines for failed
// or stopped runs and the reply being written. Tool
// messages supply code-cell results, behind the optional code disclosure.

import 'dart:convert';

import 'agent_store.dart';

sealed class ChatItem {
  const ChatItem(this.id);

  /// Stable within a chat (widget keys).
  final String id;
}

class UserText extends ChatItem {
  const UserText(super.id, this.text, {this.context});
  final String text;
  final AgentMessageContext? context;
}

/// One assistant turn, with human-facing work and edits above code details.
class AgentTurn extends ChatItem {
  const AgentTurn(
    super.id, {
    required this.texts,
    required this.cells,
    required this.activity,
    required this.edits,
    this.run,
    this.streaming = '',
  });
  final List<AssistantText> texts;
  final List<CodeCell> cells;
  final List<AgentToolActivity> activity;
  final List<AgentEdit> edits;
  final AgentRunRecord? run;
  final String streaming;
  bool get live => run != null && runActive(run!);
}

int _compareAt(String a, String b) {
  final left = DateTime.tryParse(a), right = DateTime.tryParse(b);
  return left != null && right != null ? left.compareTo(right) : a.compareTo(b);
}

/// The key a grouped turn's piece takes for a segment: the turn's id alone
/// when the chat has no outline, else the id and the segment.
String segmentedId(String turnId, String? segment) =>
    segment == null ? turnId : '$turnId#$segment';

/// The piece that holds tool call [callId], when that piece belongs to [run].
String? _ownerOf(
  Map<String, String> callTurn,
  Map<String, String> pieceRun,
  String callId,
  String run,
) {
  final pid = callTurn[callId];
  return pid != null && pieceRun[pid] == run ? pid : null;
}

/// Snapshot messages carry no run id. Tool call ids provide exact ownership;
/// paragraphs belong to the latest run that had started at their timestamp.
///
/// [anchorOf], when given, is filled with each message id that the transcript
/// shows, mapped to the item id that holds it: a user message to its own
/// bubble, an assistant message or its tool results to the turn that groups
/// them. Messages that show nothing are left out.
///
/// [segmentOf], when given, maps message ids to their outline segment (see
/// `outlineSegments`). A turn whose messages fall in more than one segment is
/// split into one item per segment, so a boundary never leaves a whole turn on
/// one side. Each piece holds its own texts and cells, its tool calls' work,
/// and the run's edits appear once, in its first piece. Without [segmentOf]
/// every run is one item.
List<ChatItem> agentTurns(
  AgentSessionState? s, {
  String streaming = '',
  String? error,
  Map<String, String>? anchorOf,
  Map<String, String>? segmentOf,
}) {
  // Items sort by time, then by position. A user message's position is its
  // index in the session; a turn's is its first message's index. A run with no
  // message takes the slots after every message, in the order it is found.
  // Replayed messages often share one timestamp, so this keeps durable order.
  final out = <(String, int, ChatItem)>[];
  void add(String at, int position, ChatItem item) =>
      out.add((at, position, item));
  if (s != null) {
    final runs = [...s.runs]
      ..sort((a, b) => _compareAt(a.startedAt, b.startedAt));
    final texts = <String, List<AssistantText>>{};
    final cells = <String, List<CodeCell>>{};
    final ats = <String, String>{};
    // Tool call id to the piece that holds its call, and each piece's run.
    final callTurn = <String, String>{};
    final pieceRun = <String, String>{};
    final pieceOrder = <String>[];
    // Each piece's position: the index of its first message.
    final piecePosition = <String, int>{};
    final users = s.messages.where((m) => m.role == 'user').toList()
      ..sort((a, b) => _compareAt(a.createdAt, b.createdAt));
    final byCall = {for (final a in s.activity) a.callId: a.runId};
    final results = {
      for (final m in s.messages)
        if (m.role == 'tool' && m.toolCallId != null) m.toolCallId!: m,
    };
    for (final (i, m) in s.messages.indexed) {
      if (m.role == 'user') {
        if (m.text.trim().isNotEmpty || m.context != null) {
          add(m.createdAt, i, UserText(m.id, m.text, context: m.context));
          anchorOf?[m.id] = m.id;
        }
      } else if (m.role == 'assistant') {
        final matched = m.toolCalls
            .map((c) => byCall[c.id])
            .whereType<String>()
            .firstOrNull;
        final r = matched == null
            ? runs
                  .where((r) => _compareAt(r.startedAt, m.createdAt) <= 0)
                  .lastOrNull
            : runs.where((r) => r.id == matched).firstOrNull;
        final id = matched ?? r?.id ?? 'message-${m.id}';
        final pid = segmentedId(id, segmentOf?[m.id]);
        if (!pieceRun.containsKey(pid)) pieceOrder.add(pid);
        piecePosition.putIfAbsent(pid, () => i);
        pieceRun[pid] = id;
        anchorOf?[m.id] = pid;
        for (final c in m.toolCalls) {
          callTurn[c.id] = pid;
        }
        if (!ats.containsKey(pid) || _compareAt(m.createdAt, ats[pid]!) < 0) {
          ats[pid] = m.createdAt;
        }
        if (m.content.trim().isNotEmpty) {
          (texts[pid] ??= []).add(AssistantText(m.id, m.content));
        }
        for (final c in m.toolCalls) {
          final result = results[c.id];
          (cells[pid] ??= []).add(
            CodeCell(
              'cell-${c.id}',
              tool: c.name,
              code: codeOf(c),
              result: result == null ? null : cellResult(result.content),
              status: result == null
                  ? (r != null && runActive(r)
                        ? CellStatus.running
                        : CellStatus.failed)
                  : (result.isError ? CellStatus.failed : CellStatus.done),
            ),
          );
        }
      }
    }
    for (final m in s.messages) {
      final turn = m.role == 'tool' ? callTurn[m.toolCallId] : null;
      if (turn != null) anchorOf?[m.id] = turn;
    }
    // Positions for pieces with no message, after every message.
    var extra = s.messages.length;
    for (final id in {
      ...runs.map((r) => r.id),
      ...pieceRun.values,
      ...s.edits.map((e) => e.runId),
      ...s.activity.map((a) => a.runId),
    }) {
      final r = runs.where((r) => r.id == id).firstOrNull;
      final activity = s.activity.where((a) => a.runId == id).toList()
        ..sort((a, b) => a.order.compareTo(b.order));
      final edits = s.edits.where((e) => e.runId == id).toList()
        ..sort((a, b) => _compareAt(a.at, b.at));
      // The run's pieces in message order; a run with no message is one piece
      // under its own id.
      final pieces = [
        for (final p in pieceOrder)
          if (pieceRun[p] == id) p,
      ];
      if (pieces.isEmpty) pieces.add(id);
      for (final pid in pieces) {
        final first = pid == pieces.first;
        var anchor = ats[pid];
        if (anchor == null && r != null) {
          final next = runs
              .where((other) => _compareAt(other.startedAt, r.startedAt) > 0)
              .firstOrNull;
          // Run creation precedes appending its user message. A queued input
          // already precedes startedAt; keep that natural order rather than
          // attaching an unrelated earlier user's timestamp to this run.
          final initiating = users
              .where(
                (m) =>
                    _compareAt(m.createdAt, r.startedAt) >= 0 &&
                    (next == null ||
                        _compareAt(m.createdAt, next.startedAt) < 0),
              )
              .firstOrNull;
          anchor = initiating?.createdAt ?? r.startedAt;
        }
        // Work goes with its tool call's piece; edits show once, in the first.
        final shownActivity = [
          for (final a in activity)
            if ((_ownerOf(callTurn, pieceRun, a.callId, id) ?? pieces.first) ==
                pid)
              a,
        ];
        final shownEdits = first ? edits : const <AgentEdit>[];
        add(
          anchor ?? edits.firstOrNull?.at ?? '',
          piecePosition[pid] ?? extra++,
          AgentTurn(
            pid,
            texts: texts[pid] ?? [],
            cells: cells[pid] ?? [],
            activity: shownActivity,
            edits: shownEdits,
            run: r,
            streaming: r != null && runActive(r) && pid == pieces.last
                ? streaming
                : '',
          ),
        );
      }
    }
  }
  out.sort((a, b) {
    final at = _compareAt(a.$1, b.$1);
    return at != 0 ? at : a.$2.compareTo(b.$2);
  });
  return [
    for (final e in out) e.$3,
    if (s == null && streaming.trim().isNotEmpty)
      AssistantText('live', streaming, live: true),
    if (error != null && error.isNotEmpty) RunNote('error', error),
  ];
}

class AssistantText extends ChatItem {
  const AssistantText(super.id, this.text, {this.live = false});
  final String text;

  /// The reply still being written.
  final bool live;
}

enum CellStatus { running, done, failed }

/// One tool call of the assistant: for `js`, its code.
class CodeCell extends ChatItem {
  const CodeCell(
    super.id, {
    required this.tool,
    required this.code,
    required this.status,
    this.result,
  });
  final String tool;
  final String code;
  final CellStatus status;
  final String? result;

  /// The first non-empty line of the code.
  String get firstLine {
    for (final l in code.split('\n')) {
      if (l.trim().isNotEmpty) return l.trim();
    }
    return tool;
  }
}

/// A run that failed or was stopped.
class RunNote extends ChatItem {
  const RunNote(super.id, this.text, {this.error = true});
  final String text;
  final bool error;
}


/// The code of a `js` call (its `code` argument), or the arguments as
/// JSON for any other tool.
String codeOf(AgentToolCall c) {
  try {
    final args = jsonDecode(c.argumentsJson);
    if (args is Map && args['code'] is String) return args['code'] as String;
  } catch (_) {}
  return c.argumentsJson;
}

/// What a `js` cell's tool message says, for people: its value or error,
/// without the runtime's note after `[---]`. When the cell returned no
/// value the message is the runtime's status JSON; then its printed
/// outputs or last error, else "No value returned."
String cellResult(String raw) {
  final cut = raw.indexOf('\n[---]\n');
  final text = (cut >= 0 ? raw.substring(0, cut) : raw).trimRight();
  try {
    final j = jsonDecode(text);
    if (j is Map && j.containsKey('execution_id') && j.containsKey('status')) {
      final outputs = [
        for (final o in (j['outputs'] as List? ?? const []))
          o is Map && o['text'] is String ? o['text'] as String : jsonEncode(o),
      ];
      if (outputs.isNotEmpty) return outputs.join('\n');
      final err = j['last_error'];
      if (err is String && err.isNotEmpty) return err;
      return 'No value returned.';
    }
  } catch (_) {}
  return text;
}

/// Pretty-printed JSON, or [raw] as it is.
String prettyJson(String raw) {
  try {
    return const JsonEncoder.withIndent('  ').convert(jsonDecode(raw));
  } catch (_) {
    return raw;
  }
}

/// The chat's items in order. [streaming] is the live reply, [error] a
/// command error.
List<ChatItem> chatItems(
  AgentSessionState? s, {
  String streaming = '',
  String? error,
}) {
  final out = <(String, int, ChatItem)>[];
  var order = 0;
  void add(String at, ChatItem item) => out.add((at, order++, item));
  if (s != null) {
    final results = <String, AgentMessage>{
      for (final m in s.messages)
        if (m.role == 'tool' && m.toolCallId != null) m.toolCallId!: m,
    };
    final active = s.runs.any(runActive);
    for (final m in s.messages) {
      switch (m.role) {
        case 'user':
          if (m.text.trim().isNotEmpty) {
            add(m.createdAt, UserText(m.id, m.text, context: m.context));
          }
        case 'assistant':
          if (m.content.trim().isNotEmpty) {
            add(m.createdAt, AssistantText(m.id, m.content));
          }
          for (final c in m.toolCalls) {
            final r = results[c.id];
            add(
              m.createdAt,
              CodeCell(
                'cell-${c.id}',
                tool: c.name,
                code: codeOf(c),
                result: r == null ? null : cellResult(r.content),
                status: r == null
                    ? (active ? CellStatus.running : CellStatus.failed)
                    : (r.isError ? CellStatus.failed : CellStatus.done),
              ),
            );
          }
      }
    }
    for (final r in s.runs) {
      final at = r.endedAt ?? r.startedAt;
      if (r.status == 'cancelled') {
        add(at, RunNote('run-${r.id}', 'Stopped.', error: false));
      } else if ((r.error ?? '').isNotEmpty) {
        add(at, RunNote('run-${r.id}', r.error!));
      }
    }
  }
  out.sort((a, b) {
    final t = a.$1.compareTo(b.$1);
    return t != 0 ? t : a.$2.compareTo(b.$2);
  });
  return [
    for (final e in out) e.$3,
    if (streaming.trim().isNotEmpty)
      AssistantText('live', streaming, live: true),
    if (error != null && error.isNotEmpty) RunNote('error', error),
  ];
}
