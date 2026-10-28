// Compacted history in the chat: the bridge's CompactedHistory mapped onto the
// kit's history cards. [compactedEntries] lists what the transcript shows in
// that mode, top to bottom: the summary cards, the "Full messages from here"
// hairline, and the live end as ordinary chat items. Nothing here hides data:
// a card's original messages are built on demand ("Show messages").

import 'package:agent_kit/agent_kit.dart'
    show
        HistoryAction,
        HistoryBoundary,
        HistoryChapter,
        HistoryChapterCard,
        HistoryController,
        HistoryCurrentChapter,
        HistoryEpisode,
        HistoryLegacy,
        HistoryLegacyCard,
        HistoryPoint,
        HistoryPointKind,
        TimelineEntry;
import 'package:flutter/widgets.dart';

import '../../src/rust/api/agent.dart'
    show
        ChapterCard,
        ChapterStatus,
        CompactedHistory,
        EpisodeAction,
        EpisodeCard,
        HistorySegmentKind,
        LegacyCard;
import 'agent_store.dart';
import 'agent_transcript.dart';

/// The id of the hairline entry.
const historyBoundaryId = 'history-boundary';

/// Whether [h] has any summary card to show. A chat with none (or one the
/// store cannot summarize) is shown in full whatever the setting says.
bool hasHistoryCards(CompactedHistory? h) =>
    h != null &&
    h.compactable &&
    h.segments.any((s) => s.kind != HistorySegmentKind.messages);

/// The segment key of every message the history covers: an episode, a legacy
/// card, or a stretch shown in full. Turns split along these keys, so a turn
/// never straddles a card edge.
Map<String, String> historySegments(CompactedHistory h) {
  final out = <String, String>{};
  void cover(int start, int end, String key) {
    for (var i = start; i < end && i < h.messageIds.length; i++) {
      out[h.messageIds[i]] = key;
    }
  }

  for (var i = 0; i < h.segments.length; i++) {
    final s = h.segments[i];
    switch (s.kind) {
      case HistorySegmentKind.legacyCard:
        final l = s.legacy;
        if (l != null) cover(l.range.start, l.range.end, 'lg:${l.id}');
      case HistorySegmentKind.chapterCard:
        final c = s.chapter;
        if (c == null) break;
        for (final e in c.episodes) {
          cover(e.range.start, e.range.end, 'ep:${e.id}');
        }
      case HistorySegmentKind.messages:
        cover(s.range.start, s.range.end, 'm:$i');
    }
  }
  return out;
}

DateTime? _time(String? s) => s == null ? null : DateTime.tryParse(s);

HistoryPointKind? _pointKind(String kind) => switch (kind) {
  'decision' => HistoryPointKind.decision,
  'correction' => HistoryPointKind.correction,
  'error' => HistoryPointKind.error,
  'question' => HistoryPointKind.question,
  _ => null,
};

HistoryAction _action(EpisodeAction a) {
  final poi = a.pointOfInterest;
  final kind = poi == null ? null : _pointKind(poi.kind);
  return HistoryAction(
    verb: a.verb,
    target: a.target,
    marker: a.changeMarker,
    point: poi == null || kind == null
        ? null
        : HistoryPoint(kind: kind, reason: poi.reason),
  );
}

HistoryEpisode historyEpisode(EpisodeCard e) => HistoryEpisode(
  id: e.id,
  title: e.title,
  gist: e.gist,
  startedAt: _time(e.startedAt),
  endedAt: _time(e.endedAt),
  messageCount: e.messageCount,
  actions: [for (final a in e.actions) _action(a)],
  actionCount: e.actionCount,
  flaggedCount: e.flaggedCount,
  errorCount: e.errorCount,
);

HistoryChapter historyChapter(ChapterCard c, int colorIndex) => HistoryChapter(
  id: c.id,
  title: c.title,
  summary: c.summary,
  startedAt: _time(c.startedAt),
  endedAt: c.status == ChapterStatus.open ? null : _time(c.endedAt),
  closed: c.status == ChapterStatus.closed,
  part: c.part_,
  parts: c.parts,
  messageCount: c.messageCount,
  episodeCount: c.episodeCount,
  episodes: [for (final e in c.episodes) historyEpisode(e)],
  colorIndex: colorIndex,
);

HistoryLegacy historyLegacy(LegacyCard l, int colorIndex) => HistoryLegacy(
  id: l.id,
  title: l.title,
  summary: l.summary,
  startedAt: _time(l.startedAt),
  endedAt: _time(l.endedAt),
  messageCount: l.messageCount,
  colorIndex: colorIndex,
);

/// The transcript in compacted mode.
class CompactedEntries {
  const CompactedEntries({required this.entries, required this.entryOf});

  /// Cards, hairline and the live end, in order.
  final List<TimelineEntry> entries;

  /// The entry that shows each message: its own turn or bubble in the live
  /// end, else the card that covers it.
  final Map<String, String> entryOf;
}

/// Builds the compacted transcript from [items] (made with
/// [historySegments] as `segmentOf`, [anchors] filled by `agentTurns`).
/// [itemView] draws one chat item; [gap] is the space between the items of an
/// opened card's original messages.
CompactedEntries compactedEntries({
  required CompactedHistory history,
  required List<ChatItem> items,
  required Map<String, String> anchors,
  required Map<String, String> segmentOf,
  required HistoryController controller,
  required Widget Function(ChatItem) itemView,
  double gap = 14,
}) {
  // Which segment key each item belongs to: a turn through its messages, a
  // bubble through its own message. Anything else (a run note, a reply still
  // being written) has none and belongs to the live end.
  final itemSegment = <String, String>{};
  anchors.forEach((messageId, itemId) {
    final key = segmentOf[messageId];
    if (key != null) itemSegment.putIfAbsent(itemId, () => key);
  });
  final byKey = <String, List<ChatItem>>{};
  final loose = <ChatItem>[];
  for (final item in items) {
    final key = itemSegment[item.id];
    if (key == null) {
      loose.add(item);
    } else {
      (byKey[key] ??= []).add(item);
    }
  }

  Widget messages(String key) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      for (final (i, item) in (byKey[key] ?? const <ChatItem>[]).indexed) ...[
        if (i > 0) SizedBox(height: gap),
        itemView(item),
      ],
    ],
  );

  final entries = <TimelineEntry>[];
  final entryOf = <String, String>{};
  void mark(Iterable<String> ids, String entry) {
    for (final id in ids) {
      entryOf.putIfAbsent(id, () => entry);
    }
  }

  // The messages of a stretch (or card) by canonical index.
  Iterable<String> idsIn(int start, int end) =>
      history.messageIds.skip(start).take(end - start);

  final messagesOf = <String, List<String>>{};
  anchors.forEach((m, item) => (messagesOf[item] ??= []).add(m));
  var color = 0;
  final colors = <String, int>{};
  final lastIndex = history.segments.length - 1;
  var cards = 0;
  for (final (i, s) in history.segments.indexed) {
    switch (s.kind) {
      case HistorySegmentKind.legacyCard:
        final l = s.legacy;
        if (l == null) break;
        final view = historyLegacy(l, color++);
        final id = 'history-legacy-${l.id}';
        mark(idsIn(l.range.start, l.range.end), id);
        entries.add(
          TimelineEntry(
            id: id,
            builder: (_) => HistoryLegacyCard(legacy: view),
          ),
        );
        cards++;
      case HistorySegmentKind.chapterCard:
        final c = s.chapter;
        if (c == null) break;
        final view = historyChapter(c, colors.putIfAbsent(c.id, () => color++));
        final id = 'history-chapter-${view.key}';
        mark(idsIn(c.range.start, c.range.end), id);
        Widget shown(BuildContext _, HistoryEpisode e) =>
            messages('ep:${e.id}');
        entries.add(
          TimelineEntry(
            id: id,
            builder: (_) => view.closed
                ? HistoryChapterCard(
                    chapter: view,
                    controller: controller,
                    messagesBuilder: shown,
                  )
                : HistoryCurrentChapter(
                    chapter: view,
                    controller: controller,
                    messagesBuilder: shown,
                  ),
          ),
        );
        cards++;
      case HistorySegmentKind.messages:
        if (i == lastIndex && cards > 0) {
          entries.add(
            TimelineEntry(
              id: historyBoundaryId,
              builder: (_) =>
                  HistoryBoundary(messageNumber: history.boundary + 1),
            ),
          );
        }
        final live = [...(byKey['m:$i'] ?? const <ChatItem>[])];
        if (i == lastIndex) live.addAll(loose);
        for (final item in live) {
          mark(messagesOf[item.id] ?? const [], item.id);
        }
        entries.addAll(live.map((item) => _entry(item, itemView)));
    }
  }
  return CompactedEntries(entries: entries, entryOf: entryOf);
}

TimelineEntry _entry(ChatItem item, Widget Function(ChatItem) itemView) =>
    TimelineEntry(id: item.id, builder: (_) => itemView(item));

/// The chat shows compacted history now: the setting is on, the store can
/// summarize this chat, and nothing is pending that needs the full transcript.
bool showsCompacted(AgentStore store, {required bool pendingReference}) =>
    store.historyCompacted(store.openId) &&
    store.compactedFor == store.openId &&
    hasHistoryCards(store.compacted) &&
    !pendingReference;
