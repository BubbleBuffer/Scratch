import 'dart:async';

import '../../core/paths.dart';
import '../../src/rust/api/agent.dart';
import 'chat_search_target.dart';
import 'outline_names.dart';

/// Search rows for agent chats: one per chat title, one per chapter and one per
/// section, written to the vault index under [agentSearchSource].
///
/// Ids are source-local and built from session and anchor ids, never titles, so
/// a rename updates its row in place. Raw messages are never indexed.
const agentSearchSource = 'agent';

/// The page type of every agent row; the target's `level` tells them apart.
const agentSearchPageType = 'chat';

/// One row as the vault index stores it. [target] is the JSON anchor.
class AgentSearchRow {
  const AgentSearchRow({
    required this.id,
    required this.title,
    required this.body,
    required this.target,
    required this.updatedMs,
  });

  final String id;
  final String title;
  final String body;
  final String target;
  final int updatedMs;
}

/// Writes rows for [agentSearchSource] into the vault index.
abstract class AgentSearchSink {
  Future<void> upsert(AgentSearchRow row);
  Future<void> delete(String id);

  /// Removes every row of [agentSearchSource].
  Future<void> clear();
}

/// Reads what the index is built from. Any failure is an error, so a missing
/// agent never looks like an empty one.
abstract class AgentSearchReader {
  Future<List<AgentSession>> sessions();
  Future<BigInt> revision(String sessionId);
  Future<ChatOutline> outline(String sessionId);
}

/// The rows a chat contributes: its shown [chatTitle], then each chapter and
/// its sections. A hub chat has no outline, so [outline] is null and only the
/// title row is made.
///
/// Labels follow the chat UI: `chat › chapter`, and `chat › chapter › section`
/// only for section rows. The title is the breadcrumb and the body is the
/// chapter or section summary. The target holds stored (raw) titles for the
/// display fallback, and the anchor ids for navigation.
List<AgentSearchRow> agentSearchRows({
  required AgentSession session,
  required String chatTitle,
  required ChatOutline? outline,
}) {
  final updatedMs =
      DateTime.tryParse(session.updatedAt)?.millisecondsSinceEpoch ?? 0;
  String target(
    String level, {
    String first = '',
    String last = '',
    String? chapterTitle,
    String? sectionTitle,
  }) => ChatSearchTarget(
    sessionId: session.id,
    firstMessageId: first,
    lastMessageId: last,
    level: level,
    chapterTitle: chapterTitle,
    sectionTitle: sectionTitle,
  ).encode();

  final rows = [
    AgentSearchRow(
      id: 'chat:${session.id}',
      title: chatTitle,
      body: '',
      target: target(chatSearchLevelChat),
      updatedMs: updatedMs,
    ),
  ];
  for (final chapter in outline?.chapters ?? const <OutlineChapter>[]) {
    final chapterName = outlineNameGist(chapter.title, chapter.summary).name;
    final chapterLabel = '$chatTitle › $chapterName';
    rows.add(
      AgentSearchRow(
        id: 'chapter:${session.id}:${chapter.firstMessageId}',
        title: chapterLabel,
        body: chapter.summary.trim(),
        target: target(
          chatSearchLevelChapter,
          first: chapter.firstMessageId,
          last: chapter.lastMessageId,
          chapterTitle: chapter.title,
        ),
        updatedMs: updatedMs,
      ),
    );
    for (final section in chapter.sections) {
      // Section labels drop a prefix that repeats the chapter name, as the
      // outline view does.
      final sectionName = outlineNameGist(
        section.title,
        '',
        chapterName: chapterName,
      ).name;
      rows.add(
        AgentSearchRow(
          id:
              'section:${session.id}:${chapter.firstMessageId}:'
              '${section.firstMessageId}',
          title: '$chapterLabel › $sectionName',
          body: section.summary.trim(),
          target: target(
            chatSearchLevelSection,
            first: section.firstMessageId,
            last: section.lastMessageId,
            chapterTitle: chapter.title,
            sectionTitle: section.title,
          ),
          updatedMs: updatedMs,
        ),
      );
    }
  }
  return rows;
}

/// Keeps [agentSearchSource] rows in step with the agent's chats.
///
/// A sweep lists the chats, reads the outline revision of each, and reads and
/// rewrites only the chats whose title, update time or revision changed. Chats
/// that left the list lose their rows. Sweeps never overlap: calls made while
/// one runs coalesce into one more, which uses the newest arguments.
///
/// Every write runs through one queue, in order. A chat removed with
/// [removeSession] is tombstoned at once, so a sweep that was already reading
/// it cannot write its rows back. The first complete sweep of a process clears
/// the source before writing, which drops rows left by an earlier run; a sweep
/// that could not read every chat clears nothing.
class AgentSearchIndex {
  AgentSearchIndex(this._sink);

  final AgentSearchSink _sink;

  /// Per chat: the key of what was last written (title, update time and
  /// revision). A sweep skips a chat whose key is unchanged.
  final Map<String, String> _keys = {};

  /// Per chat: the row ids this index has written and not yet deleted.
  final Map<String, Set<String>> _rows = {};

  /// Chats removed by [removeSession]; nothing writes their rows again.
  final Set<String> _removed = {};

  Future<void> _queue = Future.value();
  bool _reconciled = false;
  bool _sweeping = false;
  bool _again = false;
  bool _closed = false;
  _Wanted? _latest;

  /// Whether a sweep has run to completion in this process.
  bool get reconciled => _reconciled;

  /// Brings the index up to date with [reader]. [outlines] is false for a hub,
  /// whose chats are indexed by title only. [chatTitle] gives the title a chat
  /// shows. Returns once this call's sweep (or the one that absorbed it) ends.
  Future<void> sync(
    AgentSearchReader reader, {
    required bool outlines,
    required String Function(AgentSession) chatTitle,
  }) async {
    if (_closed) return;
    _latest = _Wanted(reader, outlines, chatTitle);
    if (_sweeping) {
      _again = true;
      return;
    }
    _sweeping = true;
    try {
      do {
        _again = false;
        final wanted = _latest!;
        await _sweep(wanted);
      } while (_again && !_closed);
    } finally {
      _sweeping = false;
    }
  }

  /// Forgets [sessionId] now and deletes its rows in queue order. Call this
  /// when a chat is deleted, before its list entry goes away.
  void removeSession(String sessionId) {
    if (_closed) return;
    _removed.add(sessionId);
    _keys.remove(sessionId);
    unawaited(_enqueue(() => _deleteRows(sessionId)));
  }

  /// Stops the index at once: no sweep reads further and no sweep write starts.
  /// A write already running finishes; [close] waits for it.
  void dispose() => _closed = true;

  /// [dispose], then waits until the write queue has drained, so the caller
  /// may close the vault or agent after it. Deletes already queued still run,
  /// so a removed chat's rows do not outlive it; queued sweep writes do not.
  Future<void> close() {
    dispose();
    return _queue;
  }

  Future<void> _sweep(_Wanted wanted) async {
    final List<AgentSession> sessions;
    try {
      sessions = await wanted.reader.sessions();
    } catch (e) {
      // Offline or closed: keep the cached rows as they are.
      AppLog.info('agent search: $e');
      return;
    }
    final reconcile = !_reconciled;
    var complete = true;
    final reads = <_Read>[];
    for (final session in sessions) {
      if (_closed) return;
      if (_removed.contains(session.id)) continue;
      try {
        final revision = wanted.outlines
            ? await wanted.reader.revision(session.id)
            : BigInt.zero;
        final title = wanted.chatTitle(session);
        final key = '$title\n${session.updatedAt}\n$revision';
        if (!reconcile && _keys[session.id] == key) continue;
        final outline = wanted.outlines
            ? await wanted.reader.outline(session.id)
            : null;
        reads.add(_Read(session, key, title, outline));
      } catch (e) {
        // The chat may have been deleted meanwhile; its rows are left alone.
        complete = false;
        AppLog.info('agent search: $e');
      }
    }
    if (_closed) return;
    final live = {for (final session in sessions) session.id};
    final clear = reconcile && complete;
    await _enqueue(() async {
      if (_closed) return;
      if (clear) {
        await _sink.clear();
        _rows.clear();
        _keys.clear();
        _reconciled = true;
      }
      // Chats that left the list, and removed chats whose earlier delete
      // failed, are deleted again; their ids stay in _rows until then.
      for (final id in _rows.keys.toList()) {
        if (_closed) return;
        if (live.contains(id) && !_removed.contains(id)) continue;
        await _tryDeleteRows(id);
      }
      for (final read in reads) {
        if (_closed) return;
        if (_removed.contains(read.session.id)) continue;
        try {
          await _writeSession(read);
        } catch (e) {
          // Its key was not stored, so the next sweep writes it again.
          AppLog.info('agent search: $e');
        }
      }
    });
  }

  Future<void> _writeSession(_Read read) async {
    final id = read.session.id;
    final rows = agentSearchRows(
      session: read.session,
      chatTitle: read.title,
      outline: read.outline,
    );
    final wanted = {for (final row in rows) row.id};
    for (final stale in (_rows[id] ?? const <String>{}).difference(wanted)) {
      if (_closed || _removed.contains(id)) return;
      await _sink.delete(stale);
      _rows[id]?.remove(stale);
    }
    for (final row in rows) {
      if (_closed || _removed.contains(id)) return;
      await _sink.upsert(row);
      (_rows[id] ??= {}).add(row.id);
    }
    _keys[id] = read.key;
  }

  /// Deletes every row this index holds for [sessionId]. An id leaves [_rows]
  /// only after its delete succeeds, so a failed one is retried by a later
  /// sweep. The error is rethrown after the first failure.
  Future<void> _deleteRows(String sessionId) async {
    final ids = _rows[sessionId];
    if (ids != null) {
      for (final id in ids.toList()) {
        await _sink.delete(id);
        ids.remove(id);
      }
      _rows.remove(sessionId);
    }
    _keys.remove(sessionId);
  }

  Future<void> _tryDeleteRows(String sessionId) async {
    try {
      await _deleteRows(sessionId);
    } catch (e) {
      AppLog.info('agent search: $e');
    }
  }

  Future<void> _enqueue(Future<void> Function() job) {
    final next = _queue.then((_) => job()).catchError((Object e) {
      AppLog.info('agent search index: $e');
    });
    _queue = next;
    return next;
  }
}

class _Wanted {
  const _Wanted(this.reader, this.outlines, this.chatTitle);

  final AgentSearchReader reader;
  final bool outlines;
  final String Function(AgentSession) chatTitle;
}

class _Read {
  const _Read(this.session, this.key, this.title, this.outline);

  final AgentSession session;
  final String key;
  final String title;
  final ChatOutline? outline;
}
