
import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../src/rust/api/agent.dart';
import '../../state/app_state.dart';
import 'agent_store.dart';
import 'chat_reference.dart';
import 'outline_names.dart';

enum ChatReferenceAvailability { resolved, missing, unavailable }

/// Current display data, loaded by session ID. Message order binds ranges;
/// titles never participate in lookup.
class ChatReferenceSnapshot {
  const ChatReferenceSnapshot({
    required this.chatTitle,
    required this.messageIds,
    required this.outline,
  });
  final String chatTitle;
  final List<String> messageIds;
  final ChatOutline outline;
}

abstract interface class ChatReferenceSource {
  bool get enabled;

  /// Null means the session is missing; failures mean it cannot be reached.
  Future<ChatReferenceSnapshot?> load(String sessionId);
}

class _SessionCache {
  _SessionCache(this.title, this.updatedAt, this.revision, this.snapshot);
  final String title;
  final String updatedAt;
  final BigInt revision;
  final ChatReferenceSnapshot snapshot;
}

/// Shares reads between references to the same chat; a revision or rename
/// invalidates display data. The source never changes the open session.
class AgentChatReferenceSource implements ChatReferenceSource {
  AgentChatReferenceSource(this.store);
  final AgentStore store;
  static final _sources = Expando<AgentChatReferenceSource>();
  static AgentChatReferenceSource of(AgentStore store) =>
      _sources[store] ??= AgentChatReferenceSource(store);
  final _cache = <String, _SessionCache>{};
  final _reads = <String, Future<ChatReferenceSnapshot?>>{};
  final _readAt = <String, DateTime>{};
  final _inflight = <String>{};
  NotesAgent? _lastAgent;

  @override
  bool get enabled => store.enabled;

  @override
  Future<ChatReferenceSnapshot?> load(String sessionId) {
    final at = _readAt[sessionId];
    final pending = _reads[sessionId];
    if (pending != null &&
        (_inflight.contains(sessionId) ||
            (at != null &&
                DateTime.now().difference(at) <
                    const Duration(milliseconds: 500)))) {
      return pending;
    }
    _readAt[sessionId] = DateTime.now();
    _inflight.add(sessionId);
    return _reads[sessionId] = _load(sessionId).whenComplete(() {
      _inflight.remove(sessionId);
    });
  }

  Future<ChatReferenceSnapshot?> _load(String sessionId) async {
    final agent = await store.ensureOpen();
    if (agent == null || !enabled) throw StateError('Agent unavailable');
    if (!identical(agent, _lastAgent)) {
      _lastAgent = agent;
      _cache.clear();
    }
    final sessions = await agent.sessions();
    final session = sessions.where((s) => s.id == sessionId).firstOrNull;
    if (session == null) {
      _cache.remove(sessionId);
      return null;
    }
    final revision = await agent.sessionOutlineRevision(sessionId: sessionId);
    final cached = _cache[sessionId];
    if (cached != null &&
        cached.title == session.title &&
        cached.updatedAt == session.updatedAt &&
        cached.revision == revision) {
      return cached.snapshot;
    }
    final state = await agent.sessionState(sessionId: sessionId);
    final outline = await agent.sessionOutline(sessionId: sessionId);
    final userNamed = (await agent.sessionUserNamedIds()).contains(sessionId);
    final snapshot = ChatReferenceSnapshot(
      chatTitle: userNamed || store.hostInfo?.kind == 'hub'
          ? state.session.title
          : outlineNameGist(state.session.title, '').name,
      messageIds: [for (final m in state.messages) m.id],
      outline: outline,
    );
    _cache[sessionId] = _SessionCache(
      session.title,
      session.updatedAt,
      revision,
      snapshot,
    );
    return snapshot;
  }
}

/// Resolves the containing current chapter/section by message range. A range
/// split by resegmentation follows its first anchor for display; the original
/// first and last anchors remain the link.
ChatReference? resolveChatReference(
  ChatReference cached,
  ChatReferenceSnapshot snapshot,
) {
  final order = {
    for (var i = 0; i < snapshot.messageIds.length; i++)
      snapshot.messageIds[i]: i,
  };
  final first = order[cached.firstMessageId],
      last = order[cached.lastMessageId];
  if (first == null || last == null || first > last) return null;
  bool contains(String start, String end) {
    final a = order[start], b = order[end];
    return a != null && b != null && a <= first && b >= last;
  }

  bool containsFirst(String start, String end) {
    final a = order[start], b = order[end];
    return a != null && b != null && a <= first && b >= first;
  }

  final chapter =
      snapshot.outline.chapters
          .where((c) => contains(c.firstMessageId, c.lastMessageId))
          .firstOrNull ??
      snapshot.outline.chapters
          .where((c) => containsFirst(c.firstMessageId, c.lastMessageId))
          .firstOrNull;
  // The range still exists but has no current outline: keep its cached
  // chapter/section display while updating the resolvable session name.
  ChatReference? current;
  if (chapter != null) {
    if (cached.isSection) {
      final section =
          chapter.sections
              .where((s) => contains(s.firstMessageId, s.lastMessageId))
              .firstOrNull ??
          chapter.sections
              .where((s) => containsFirst(s.firstMessageId, s.lastMessageId))
              .firstOrNull;
      if (section != null) {
        current = ChatReference.fromSection(
          sessionId: cached.sessionId,
          chatTitle: snapshot.chatTitle,
          chapter: chapter,
          section: section,
        );
      }
    } else {
      current = ChatReference.fromChapter(
        sessionId: cached.sessionId,
        chatTitle: snapshot.chatTitle,
        chapter: chapter,
      );
    }
  }
  current ??= cached;
  return ChatReference(
    level: cached.level,
    sessionId: cached.sessionId,
    firstMessageId: cached.firstMessageId,
    lastMessageId: cached.lastMessageId,
    chatTitle: snapshot.chatTitle,
    chapterTitle: current.chapterTitle,
    sectionTitle: current.sectionTitle,
    summary: current.summary,
    decision: current.decision,
    when: current.when,
    colour: current.colour,
    sections: current.sections,
  );
}

/// Resolves visible references and emits cache updates only when data changes.
/// Its bounded refresh timer lives only as long as this mounted reference.
class ResolvedChatReference extends StatefulWidget {
  const ResolvedChatReference({
    super.key,
    required this.reference,
    required this.builder,
    this.source,
    this.onCacheChanged,
  });
  final ChatReference reference;
  final ChatReferenceSource? source;
  final ValueChanged<ChatReference>? onCacheChanged;
  final Widget Function(
    BuildContext context,
    ChatReference reference,
    ChatReferenceAvailability availability,
  )
  builder;

  @override
  State<ResolvedChatReference> createState() => _ResolvedChatReferenceState();
}

class _ResolvedChatReferenceState extends State<ResolvedChatReference> {
  late ChatReference _reference = widget.reference;
  ChatReferenceAvailability _availability =
      ChatReferenceAvailability.unavailable;
  ChatReferenceSource? _source;
  AgentStore? _store;
  Timer? _timer;
  bool _loading = false;
  int _generation = 0;
  String? _lastEmitted;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final app = AppScope.maybeOf(context);
    final store = app == null ? null : AgentStore.of(app);
    if (!identical(store, _store)) {
      _store?.removeListener(_changed);
      _store = store;
      _store?.addListener(_changed);
    }
    _source =
        widget.source ??
        (store == null ? null : AgentChatReferenceSource.of(store));
    _changed();
  }

  @override
  void didUpdateWidget(ResolvedChatReference oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.reference.toTarget() != widget.reference.toTarget() ||
        !identical(oldWidget.source, widget.source)) {
      _generation++;
      _lastEmitted = null;
      _reference = widget.reference;
      _source =
          widget.source ??
          (_store == null ? null : AgentChatReferenceSource.of(_store!));
      _changed();
    }
  }

  void _changed() {
    if (_source?.enabled ?? false) {
      _timer ??= Timer.periodic(const Duration(seconds: 1), (_) => _changed());
    } else {
      _timer?.cancel();
      _timer = null;
    }
    // Store notifications can arrive during a parent build.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) unawaited(_resolve());
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  Future<void> _resolve() async {
    final source = _source;
    final generation = _generation;
    if (source == null || !source.enabled) {
      _generation++;
      if (_reference.toTarget() != widget.reference.toTarget() ||
          _availability != ChatReferenceAvailability.unavailable) {
        setState(() {
          _reference = widget.reference;
          _availability = ChatReferenceAvailability.unavailable;
        });
      }
      return;
    }
    if (_loading) return;
    _loading = true;
    ChatReference next = widget.reference;
    var availability = ChatReferenceAvailability.unavailable;
    try {
      final snapshot = await source.load(widget.reference.sessionId);
      if (snapshot == null) {
        availability = ChatReferenceAvailability.missing;
      } else {
        final resolved = resolveChatReference(widget.reference, snapshot);
        if (resolved == null) {
          availability = ChatReferenceAvailability.missing;
        } else {
          next = resolved;
          availability = ChatReferenceAvailability.resolved;
        }
      }
    } catch (_) {
      // An unreachable agent keeps a useful cached reference.
    } finally {
      _loading = false;
    }
    if (!mounted || generation != _generation || !source.enabled) return;
    if (next.toTarget() != _reference.toTarget() ||
        availability != _availability) {
      setState(() {
        _reference = next;
        _availability = availability;
      });
    }
    final payload = next.toTarget();
    if (availability == ChatReferenceAvailability.resolved &&
        payload != widget.reference.toTarget() &&
        payload != _lastEmitted) {
      _lastEmitted = payload;
      widget.onCacheChanged?.call(next);
    }
  }

  @override
  void dispose() {
    _generation++;
    _timer?.cancel();
    _store?.removeListener(_changed);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) =>
      widget.builder(context, _reference, _availability);
}
