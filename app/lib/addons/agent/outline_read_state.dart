// What the reader has done to the outline of one chat: which chapters they
// opened, expanded or left open under a "Chapter closed" heading, which are
// collapsing, and the stable key of every row of the last layout.

import 'package:flutter/widgets.dart';

import 'agent_outline.dart';
import 'agent_store.dart';

class OutlineReadState {
  OutlineReadState({
    required this.vsync,
    required this.mounted,
    required this.update,
    required this.pinTop,
  });

  /// Drives the collapse animations.
  final TickerProvider vsync;

  /// Whether the chat is still on screen, and the chat's `setState`.
  final bool Function() mounted;
  final void Function(VoidCallback) update;

  /// Pins the list to the row at the top of the viewport.
  final VoidCallback pinTop;

  /// Chapters whose messages the reader has shown, keyed by the chapter's first
  /// message id, which outlives the chapter's id. Explicit states only: false
  /// is an explicit fold, true an explicit unfold. Others follow the default.
  final shown = <String, bool>{};
  final expanded = <String, bool>{};

  /// Chapters that closed while the reader was inside them. They stay open,
  /// under a quiet "Chapter closed · Fold" heading, until the reader scrolls
  /// away or taps Fold. Keyed by first message id.
  final deferred = <String>{};

  /// Chapters whose messages are collapsing, by first message id. Each runs
  /// its own animation from open to closed and hides the messages at its end.
  final collapsing = <String, AnimationController>{};

  /// The chapter, by first message id, of each item row laid out so far, so a
  /// folded item can rebase on its chapter's card.
  final itemChapter = <String, String>{};

  /// First message ids of the chapters seen open, so a close is noticed.
  final openFirsts = <String>{};

  /// The stable key of each row of the last build, and its index. The list
  /// rebases on these, so a row keeps its place across outline changes.
  List<String> stableKeys = const [];
  Map<String, int> stableIndex = const {};

  static const collapseDuration = Duration(milliseconds: 240);
  static const full = AlwaysStoppedAnimation<double>(1);

  /// Forgets everything: another chat is open.
  void reset() {
    shown.clear();
    expanded.clear();
    deferred.clear();
    openFirsts.clear();
    itemChapter.clear();
    for (final c in collapsing.values) {
      c.dispose();
    }
    collapsing.clear();
  }

  void dispose() {
    for (final c in collapsing.values) {
      c.dispose();
    }
  }

  /// A closed chapter's messages are hidden until shown. An open chapter's
  /// are always shown.
  bool hidden(ChapterView chapter) =>
      chapter.foldedByDefault && !(shown[chapter.first] ?? false);

  /// Shows or hides [chapter]'s messages by hand. Call inside `setState`.
  void show(ChapterView chapter, bool value) {
    cancelCollapse(chapter.first);
    shown[chapter.first] = value;
  }

  /// Notices the chapters that closed since the last build. One that closed
  /// while the reader is inside it stays open under its "Chapter closed · Fold"
  /// heading. Any other is shown for one frame and then folds, so its card
  /// animates the hide. [inView] holds the first message ids of the chapters
  /// on screen; [anchorId] is the message at the top of the reader's view.
  void closeFolds(
    OutlineView outline,
    Set<String> inView,
    List<AgentMessage> messages,
    String? anchorId,
  ) {
    // A live update can extend the chapter and close it together. Classify
    // the visible message against its new anchors as well as the old rows.
    final position = messages.indexWhere((m) => m.id == anchorId);
    if (position >= 0) {
      final order = {
        for (var i = 0; i < messages.length; i++) messages[i].id: i,
      };
      for (final c in outline.chapters) {
        final first = order[c.first], last = order[c.last];
        if (first != null &&
            last != null &&
            first <= position &&
            position <= last) {
          inView.add(c.first);
        }
      }
    }
    for (final c in outline.chapters) {
      if (c.ongoing) {
        openFirsts.add(c.first);
        continue;
      }
      if (!openFirsts.remove(c.first)) continue;
      shown[c.first] = true;
      if (inView.contains(c.first)) {
        deferred.add(c.first);
      } else {
        foldAfterFrame(c.first);
      }
    }
  }

  /// Folds [first]'s messages on the next frame, once the card has shown them.
  void foldAfterFrame(String first) {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted() || deferred.contains(first) || shown[first] != true) {
        return;
      }
      _collapse(first);
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  /// Collapses [first]'s message rows over [collapseDuration], then hides
  /// them. The list is first pinned to the row at the top of the viewport, so
  /// rows above it shrinking cannot move what the reader is looking at.
  void _collapse(String first) {
    if (collapsing.containsKey(first)) return;
    pinTop();
    final animation = AnimationController(
      vsync: vsync,
      duration: collapseDuration,
    );
    update(() => collapsing[first] = animation);
    animation.reverse(from: 1).whenComplete(() {
      if (!mounted() || !identical(collapsing[first], animation)) return;
      update(() {
        collapsing.remove(first)?.dispose();
        shown.remove(first);
      });
    });
  }

  /// Stops a collapse in progress when the reader shows or folds the chapter
  /// by hand.
  void cancelCollapse(String first) => collapsing.remove(first)?.dispose();

  /// Ends the deferred state of [first] and folds its messages.
  void foldNow(String first) {
    update(() => deferred.remove(first));
    foldAfterFrame(first);
  }

  /// Folds the deferred chapters the reader has scrolled away from.
  void foldAwayFromDeferred(Set<String> Function() inViewNow) {
    if (deferred.isEmpty) return;
    final inView = inViewNow();
    for (final first in deferred.toList()) {
      if (!inView.contains(first)) foldNow(first);
    }
  }

  static String chapterKey(ChapterView c) => 'chap:first:${c.first}';

  static String sectionKey(SectionView s) => 'sec:first:${s.first}';

  /// Stable row keys use the first message carried by each turn piece, so a
  /// section inserted earlier cannot change a later piece's identity.
  List<String> stableKeysOf(OutlineLayout layout, OutlineView outline) {
    final pieces = <String, int>{};
    final keys = <String>[];
    for (final row in layout.rows) {
      switch (row) {
        case ItemRow(:final item):
          final turn =
              layout.firstMessageFor(item.id) ?? item.id.split('#').first;
          final n = pieces[turn] ?? 0;
          pieces[turn] = n + 1;
          final key = n == 0 ? 'item:$turn' : 'item:$turn~$n';
          itemChapter[key] = row.chapter >= 0
              ? outline.chapters[row.chapter].first
              : '';
          keys.add(key);
        case ChapterRow():
          keys.add(chapterKey(outline.chapters[row.chapter]));
        case SectionRow():
          keys.add(
            sectionKey(outline.chapters[row.chapter].sections[row.section]),
          );
      }
    }
    return keys;
  }

  /// Keys the rows of [layout], and indexes the keys.
  void rekey(OutlineLayout layout, OutlineView outline) {
    stableKeys = stableKeysOf(layout, outline);
    stableIndex = {
      for (var i = 0; i < stableKeys.length; i++) stableKeys[i]: i,
    };
  }

  /// The row index of a stable key. An item that is folded away maps to its
  /// chapter's card, so the list rebases there rather than losing its place.
  int? indexOfKey(String key) {
    final direct = stableIndex[key];
    if (direct != null || !key.startsWith('item:')) return direct;
    final first = itemChapter[key];
    if (first == null || first.isEmpty) return null;
    return stableIndex['chap:first:$first'];
  }
}
