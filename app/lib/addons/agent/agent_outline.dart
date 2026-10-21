// The chat outline laid over the transcript: chapters and their sections as
// rows between the chat's items. A closed chapter folds to one card and its
// items leave the rows, so a long chat builds only what is shown. Message ids
// anchor everything: a chapter or section covers the messages from its first
// to its last id, and a grouped turn answers for each message it holds.
//
// Pure Dart on purpose: the widgets live in agent_outline_view.dart.

import 'dart:math';

import '../../src/rust/api/agent.dart';
import 'agent_transcript.dart';
import 'outline_names.dart';

/// The outline as the chat uses it, with the message counts derived from the
/// session's message ids (the outline itself carries none).
class OutlineView {
  const OutlineView(this.chapters);

  static const empty = OutlineView([]);

  /// Builds the view of [outline]. [messages] are the session's messages in
  /// order; a chapter's message count is its span's messages that are not tool
  /// results.
  factory OutlineView.of(
    ChatOutline outline, {
    List<AgentMessage> messages = const [],
  }) {
    final pos = <String, int>{
      for (var i = 0; i < messages.length; i++) messages[i].id: i,
    };
    return OutlineView([
      for (final c in outline.chapters)
        ChapterView(
          id: c.id,
          title: c.title,
          first: c.firstMessageId,
          last: c.lastMessageId,
          decisions: c.decisions,
          summary: c.summary,
          when: _when(c.startedAt, c.ongoing ? null : c.endedAt),
          ongoing: c.ongoing,
          messageCount: _countMessages(
            pos,
            messages,
            c.firstMessageId,
            c.lastMessageId,
          ),
          sections: [
            for (final s in c.sections)
              SectionView(
                id: s.id,
                title: s.title,
                first: s.firstMessageId,
                last: s.lastMessageId,
                chapterName: outlineNameGist(c.title, c.summary).name,
              ),
          ],
        ),
    ]);
  }

  final List<ChapterView> chapters;
  bool get isEmpty => chapters.isEmpty;
}

class ChapterView {
  const ChapterView({
    required this.id,
    required String title,
    required this.first,
    required this.last,
    this.decisions = const [],
    this.summary = '',
    this.when = '',
    this.ongoing = false,
    this.messageCount = 0,
    this.sections = const [],
  }) : rawTitle = title;

  final String id, rawTitle;
  String get title => outlineNameGist(rawTitle, summary).name;

  /// The first and last message ids of the chapter, inclusive.
  final String first, last;
  final List<String> decisions;
  final String summary;

  /// A date range for people, such as "Sep 28 – 30" or "since Oct 6".
  final String when;

  /// An open chapter: it stays unfolded as a heading and cannot fold.
  final bool ongoing;

  /// The chapter's messages that are not tool results.
  final int messageCount;
  final List<SectionView> sections;

  /// Closed chapters start as a card with their messages hidden.
  bool get foldedByDefault => !ongoing;

  /// The first non-blank decision, the one line a card shows.
  String? get decision => outlineDecision(decisions);

  /// The line a card shows when there is no decision: the summary's first
  /// sentence, or null when there is no summary.
  String? get gist {
    final gist = outlineNameGist(rawTitle, summary).gist;
    return gist.isEmpty ? null : gist;
  }
}

class SectionView {
  const SectionView({
    required this.id,
    required String title,
    required this.first,
    required this.last,
    this.chapterName,
  }) : rawTitle = title;

  final String id, rawTitle;
  final String? chapterName;
  String get title =>
      outlineNameGist(rawTitle, '', chapterName: chapterName).name;
  final String first, last;
}

const _months = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

/// "Sep 28 – 30" within a month, "Sep 28 – Oct 2" across two, "since Oct 6"
/// while open ([endedAt] null). Empty when the start is not a date.
String _when(String startedAt, String? endedAt) {
  final start = DateTime.tryParse(startedAt)?.toLocal();
  if (start == null) return '';
  final from = '${_months[start.month - 1]} ${start.day}';
  if (endedAt == null) return 'since $from';
  final end = DateTime.tryParse(endedAt)?.toLocal();
  if (end == null) return from;
  if (end.year == start.year &&
      end.month == start.month &&
      end.day == start.day) {
    return from;
  }
  if (end.year == start.year && end.month == start.month) {
    return '$from – ${end.day}';
  }
  return '$from – ${_months[end.month - 1]} ${end.day}';
}

int _countMessages(
  Map<String, int> pos,
  List<AgentMessage> messages,
  String first,
  String last,
) {
  final span = _span(pos, first, last);
  if (span == null) return 0;
  var n = 0;
  for (var p = span.$1; p <= span.$2; p++) {
    if (messages[p].role != 'tool') n++;
  }
  return n;
}

/// One line of the transcript: a chat item, a chapter's card or heading, or a
/// section's divider. [chapter] and [section] are indexes into the outline
/// (-1 when outside), which is what the where-am-I strip reads.
sealed class OutlineRow {
  const OutlineRow(this.key, {required this.chapter, required this.section});

  /// Stable across rebuilds; scroll targets and widget keys use it.
  final String key;
  final int chapter;
  final int section;
}

class ItemRow extends OutlineRow {
  const ItemRow(
    super.key,
    this.item, {
    required super.chapter,
    required super.section,
  });
  final ChatItem item;
}

class ChapterRow extends OutlineRow {
  const ChapterRow(super.key, {required super.chapter, required this.folded})
    : super(section: -1);

  /// True when the chapter's messages are hidden (the card shows alone), false
  /// when they show under it or the chapter is open.
  final bool folded;
}

class SectionRow extends OutlineRow {
  const SectionRow(super.key, {required super.chapter, required super.section});
}

/// Where a jump to a span lands: the item that holds its first anchored
/// message, and the chapter that item sits in (-1 when none).
class OutlineTarget {
  const OutlineTarget(this.itemId, this.chapter);
  final String itemId;
  final int chapter;
}

/// Each message's segment: the chapter and section it sits in, as a key the
/// transcript splits grouped turns by. Messages outside every chapter share
/// one segment. Empty when [outline] is empty, so the transcript is unsplit.
Map<String, String> outlineSegments(
  OutlineView outline,
  List<String> messageIds,
) {
  if (outline.isEmpty) return const {};
  final pos = <String, int>{
    for (var i = 0; i < messageIds.length; i++) messageIds[i]: i,
  };
  final chapters = outline.chapters;
  final chapterSpans = [for (final c in chapters) _span(pos, c.first, c.last)];
  final sectionSpans = [
    for (final c in chapters)
      [for (final s in c.sections) _span(pos, s.first, s.last)],
  ];
  return {
    for (var p = 0; p < messageIds.length; p++)
      messageIds[p]: () {
        final ch = _chapterAt(chapterSpans, p);
        final sec = _sectionAt(sectionSpans, ch, p);
        return ch < 0 ? '-' : '$ch.$sec';
      }(),
  };
}

/// The transcript's rows for one outline and fold state.
class OutlineLayout {
  OutlineLayout._(
    this.rows,
    this._rowOf,
    this._itemChapter,
    this._messageIds,
    this._pos,
    this._anchorOf,
  );

  final List<OutlineRow> rows;
  final Map<String, int> _rowOf;
  final Map<String, int> _itemChapter;
  final List<String> _messageIds;
  final Map<String, int> _pos;
  final Map<String, String> _anchorOf;

  /// Builds the rows. [items] are the transcript items in order, [messageIds]
  /// the session's message ids in order, and [anchorOf] maps each message id
  /// to the item that shows it (see `agentTurns`). [folded] says whether a
  /// chapter's messages are hidden right now.
  factory OutlineLayout.build({
    required OutlineView outline,
    required List<ChatItem> items,
    required List<String> messageIds,
    required Map<String, String> anchorOf,
    required bool Function(ChapterView chapter) folded,
  }) {
    final pos = <String, int>{
      for (var i = 0; i < messageIds.length; i++) messageIds[i]: i,
    };
    final chapters = outline.chapters;
    final chapterSpans = [
      for (final c in chapters) _span(pos, c.first, c.last),
    ];
    final sectionSpans = [
      for (final c in chapters)
        [for (final s in c.sections) _span(pos, s.first, s.last)],
    ];

    // Each item's first and last message position. Items that show no
    // message (run notes, approvals, the live reply) take the position of the
    // item before them, so they stay with their context.
    final lo = <String, int>{}, hi = <String, int>{};
    anchorOf.forEach((msg, item) {
      final p = pos[msg];
      if (p == null) return;
      final l = lo[item];
      if (l == null || p < l) lo[item] = p;
      final h = hi[item];
      if (h == null || p > h) hi[item] = p;
    });

    final rows = <OutlineRow>[];
    final itemChapter = <String, int>{};
    var prev = -1;
    var curChapter = -1, curSection = -1;
    for (final item in items) {
      // An item that the transcript placed before an earlier one stays after
      // it, so the rows never return to a chapter or section they left.
      final l = max(lo[item.id] ?? prev, prev);
      final h = max(hi[item.id] ?? l, l);
      prev = l;
      final ch = l < 0 ? -1 : _chapterAt(chapterSpans, l);
      final sec = _sectionAt(sectionSpans, ch, l);
      itemChapter[item.id] = ch;
      if (ch != curChapter) {
        curChapter = ch;
        curSection = -1;
        if (ch >= 0) {
          final c = chapters[ch];
          rows.add(ChapterRow('chap:${c.id}', chapter: ch, folded: folded(c)));
        }
      }
      // An item is hidden only when every message it holds is inside a folded
      // chapter. A turn that runs into the next chapter stays visible.
      final span = ch >= 0 ? chapterSpans[ch] : null;
      final hidden =
          span != null && folded(chapters[ch]) && l >= span.$1 && h <= span.$2;
      if (hidden) continue;
      if (ch >= 0 && !folded(chapters[ch]) && sec >= 0 && sec != curSection) {
        rows.add(
          SectionRow(
            'sec:${chapters[ch].sections[sec].id}',
            chapter: ch,
            section: sec,
          ),
        );
      }
      curSection = sec;
      rows.add(ItemRow('item:${item.id}', item, chapter: ch, section: sec));
    }

    final rowOf = <String, int>{
      for (var i = 0; i < rows.length; i++) rows[i].key: i,
    };
    return OutlineLayout._(rows, rowOf, itemChapter, messageIds, pos, anchorOf);
  }

  /// The row index for a row key, or null when that row is not shown.
  int? rowIndex(String key) => _rowOf[key];

  static String itemKey(String itemId) => 'item:$itemId';

  /// First message carried by an item, for preserving a plain chat's place.
  String? firstMessageFor(String itemId) => _firstByItem[itemId];

  late final Map<String, String> _firstByItem = () {
    final first = <String, String>{};
    for (final id in _messageIds) {
      final item = _anchorOf[id];
      if (item != null) first.putIfAbsent(item, () => id);
    }
    return first;
  }();

  /// The jump target for the span [first]..[last]: the item of its first
  /// message that is shown in the transcript (a message with no shown item is
  /// skipped), or null when no message in the span is shown.
  OutlineTarget? target(String first, String last) {
    final span = _span(_pos, first, last);
    if (span == null) return null;
    for (var p = span.$1; p <= span.$2; p++) {
      final item = _anchorOf[_messageIds[p]];
      if (item != null && _itemChapter.containsKey(item)) {
        return OutlineTarget(item, _itemChapter[item]!);
      }
    }
    return null;
  }

  /// The row keys of the items that hold messages in the span, for flashing
  /// the target after a jump.
  Set<String> spanKeys(String first, String last) {
    final span = _span(_pos, first, last);
    if (span == null) return {};
    return {
      for (var p = span.$1; p <= span.$2; p++)
        if (_anchorOf[_messageIds[p]] case final item?) itemKey(item),
    };
  }

  /// The chapter an item sits in (-1 when none), for hidden items too.
  int chapterOfItem(String itemId) => _itemChapter[itemId] ?? -1;
}

/// The first and last messages' positions, ordered, or null when neither id is
/// in the session.
(int, int)? _span(Map<String, int> pos, String first, String last) {
  final a = pos[first] ?? pos[last], b = pos[last] ?? pos[first];
  if (a == null || b == null) return null;
  return a <= b ? (a, b) : (b, a);
}

/// The index of the chapter whose span holds position [p], or -1.
int _chapterAt(List<(int, int)?> spans, int p) {
  for (var i = 0; i < spans.length; i++) {
    final s = spans[i];
    if (s != null && p >= s.$1 && p <= s.$2) return i;
  }
  return -1;
}

/// The index of section [ch]'s span that holds position [p], or -1.
int _sectionAt(List<List<(int, int)?>> sectionSpans, int ch, int p) {
  if (ch < 0) return -1;
  final spans = sectionSpans[ch];
  for (var j = 0; j < spans.length; j++) {
    final s = spans[j];
    if (s != null && p >= s.$1 && p <= s.$2) return j;
  }
  return -1;
}
