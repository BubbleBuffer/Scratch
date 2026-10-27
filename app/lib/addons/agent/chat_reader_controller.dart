// Where the reader is in a chat and how it moves: the two lists' scroll
// positions, following the end of a live reply, keeping the message at the top
// of the view when the outline changes, and the jumps to a span, a chapter or
// a pending reference.

import 'package:flutter/widgets.dart';

import '../../src/rust/api/agent.dart' show ChatOutline;
import '../../ui/anchored_list.dart';
import 'agent_outline.dart';
import 'agent_outline_view.dart';
import 'agent_store.dart';
import 'outline_read_state.dart';

class ChatReaderController {
  ChatReaderController({
    required this.read,
    required this.store,
    required this.presentation,
    required this.mounted,
    required this.update,
  });

  final OutlineReadState read;

  /// The chat's store and presentation as they are now, whether the chat is
  /// still on screen, and the chat's `setState`.
  final AgentStore Function() store;
  final AgentPresentation Function() presentation;
  final bool Function() mounted;
  final void Function(VoidCallback) update;

  /// The outlined transcript, laid out by row index with message anchors.
  final list = AnchoredListController();

  /// The plain list: a chat with no outline and no pending reference. It keeps
  /// its scroll controller and its spacing until the first outline arrives.
  final scroll = ScrollController();

  /// Whether the rows are laid out by index. Latches on: once an outline or a
  /// pending reference arrives, the chat stays on the indexed list.
  bool indexed = false;

  /// The plain list's rows by index, and its viewport, so the top row can be
  /// measured when the chat switches to the indexed list.
  final plainRows = <int, GlobalKey>{};
  final plainViewport = GlobalKey();
  final transcriptViewport = GlobalKey();
  ChatOutline? rawOutline;
  int positionRestore = 0;
  ({String id, double y})? readingAnchor;
  bool _readerMeasureScheduled = false;

  /// Where the indexed list starts after the switch: the outline row of the
  /// plain list's top row, and that row's top edge in the viewport.
  ({int index, double y, double globalY})? restore;

  /// The chat the fold and place state belong to. Another chat resets it.
  String? stateFor;

  final where = ValueNotifier<OutlineWhere>((
    chapter: null,
    section: null,
    chapterId: null,
    sectionId: null,
  ));

  /// The row index at the top of the transcript, as the list reports it, and
  /// the first and last visible rows.
  int? _top;
  (int, int)? visibleRange;

  int lastMessages = 0;
  String last = '';

  /// The outline and layout of the last build. Jumps and the strip read them.
  OutlineView outline = OutlineView.empty;
  OutlineLayout? layout;

  /// The rows a jump flashes, and which jump it was.
  Set<String> flashKeys = {};
  int flashToken = 0;
  bool _taking = false;

  void dispose() {
    list.dispose();
    scroll.dispose();
    where.dispose();
  }

  /// Forgets the place in the last chat: another chat is open.
  void reset() {
    indexed = false;
    restore = null;
    plainRows.clear();
    flashKeys = {};
    _top = null;
    visibleRange = null;
    rawOutline = null;
    positionRestore++;
    readingAnchor = null;
    lastMessages = 0;
    last = '';
  }

  /// Whether the reader is at the end of the transcript. Only then does a new
  /// reply follow the end; a reader who sent a message always goes to it.
  bool get atEnd =>
      !list.scroll.hasClients || list.scroll.position.extentAfter < 24;

  /// Whether the plain list shows its last row, or is not yet laid out.
  bool get plainAtEnd => !scroll.hasClients || scroll.position.extentAfter < 24;

  /// Follows the end when a reply or a message arrives. The outline's own
  /// changes do not count, and a reader away from the end keeps their place
  /// unless they sent the message.
  void follow(AgentStore s) {
    final messages = s.state?.messages ?? const <AgentMessage>[];
    final token =
        '${s.openId}:${messages.length}:${s.streaming}:${s.state?.edits.length}:${s.state?.activity.length}';
    if (last == token) return;
    final sent =
        messages.length > lastMessages &&
        messages.lastOrNull?.role == 'user' &&
        !(s.replay?.active == true && s.replay?.sessionId == s.openId);
    last = token;
    lastMessages = messages.length;
    if (!sent && !(indexed ? atEnd : plainAtEnd)) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      // A pending reference jumps instead of following the end.
      if (!mounted() || store().pendingFor(presentation()) != null) return;
      if (indexed) {
        list.jumpToEnd();
      } else if (scroll.hasClients) {
        scroll.jumpTo(scroll.position.maxScrollExtent);
      }
    });
  }

  /// Shows a pending reference once its chat and outline have both loaded.
  void maybeTakePending() {
    if (store().pendingFor(presentation()) == null || _taking) return;
    _taking = true;
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      try {
        if (!mounted()) return;
        final s = store();
        final p = s.pendingFor(presentation());
        if (p == null ||
            s.state?.session.id != p.sessionId ||
            s.outlineFor != p.sessionId) {
          return;
        }
        s.takePendingAnchor(presentation());
        await jump(p.firstMessageId, p.lastMessageId);
      } finally {
        _taking = false;
      }
    });
  }

  Future<void> toggleMessages(ChapterView chapter, bool shown) async {
    positionRestore++;
    update(() => read.show(chapter, shown));
    await WidgetsBinding.instance.endOfFrame;
    if (!mounted()) return;
    final index = layout?.rowIndex('chap:${chapter.id}');
    if (index != null) list.jumpToIndex(index);
  }

  /// Shows the span from [first] to [last]. A hidden chapter is shown first,
  /// then the list jumps by row index to the span's first shown row (the
  /// section's divider when [sectionKey] names one) and flashes the span.
  Future<void> jump(String first, String last, {String? sectionKey}) async {
    positionRestore++;
    var layout = this.layout;
    var target = layout?.target(first, last);
    if (layout == null || target == null) return;
    if (target.chapter >= 0 && read.hidden(outline.chapters[target.chapter])) {
      final hidden = outline.chapters[target.chapter];
      update(() => read.show(hidden, true));
      await WidgetsBinding.instance.endOfFrame;
      if (!mounted()) return;
      layout = this.layout;
      target = layout?.target(first, last);
      if (layout == null || target == null) return;
    }
    final index =
        (sectionKey == null ? null : layout.rowIndex(sectionKey)) ??
        layout.rowIndex(OutlineLayout.itemKey(target.itemId));
    if (index == null) return;
    update(() {
      flashKeys = layout!.spanKeys(first, last);
      flashToken++;
    });
    list.jumpToIndex(index);
  }

  /// Shows a chapter from the contents: a closed chapter's messages are shown
  /// under its card, and the list jumps to the card or heading.
  Future<void> openChapter(ChapterView chapter) async {
    positionRestore++;
    final key = 'chap:${chapter.id}';
    if (read.hidden(chapter)) {
      update(() => read.show(chapter, true));
      await WidgetsBinding.instance.endOfFrame;
      if (!mounted()) return;
    }
    final index = layout?.rowIndex(key);
    if (index == null) return;
    update(() {
      flashKeys = {key};
      flashToken++;
    });
    list.jumpToIndex(index);
  }

  /// Called by the list when the visible rows change: reads the chapter and
  /// section at the top into the where-am-I strip, tracks whether the reader
  /// is at the end, and folds deferred chapters the reader has left.
  void onVisible(int first, int last) {
    _top = first;
    visibleRange = (first, last);
    refreshWhere();
    read.foldAwayFromDeferred(chaptersInView);
  }

  void refreshWhere() {
    final layout = this.layout;
    final top = _top;
    final OutlineRow? row =
        layout != null && top != null && top < layout.rows.length
        ? layout.rows[top]
        : null;
    final chapter = row == null || row.chapter < 0
        ? null
        : outline.chapters[row.chapter];
    final section = chapter == null || row!.section < 0
        ? null
        : chapter.sections[row.section];
    final next = (
      chapter: chapter?.title,
      section: section?.title,
      chapterId: chapter?.id,
      sectionId: section?.id,
    );
    if (where.value != next) where.value = next;
  }

  /// The first message ids of the chapters with a row on screen, in the last
  /// build's layout.
  Set<String> chaptersInView() {
    final layout = this.layout, range = visibleRange;
    if (layout == null || range == null || layout.rows.isEmpty) return {};
    final last = range.$2 < layout.rows.length
        ? range.$2
        : layout.rows.length - 1;
    final out = <String>{};
    for (var i = range.$1; i <= last; i++) {
      final ch = layout.rows[i].chapter;
      if (ch >= 0 && ch < outline.chapters.length) {
        out.add(outline.chapters[ch].first);
      }
    }
    return out;
  }

  /// The plain list's top row, as an index of the outline layout and the y of
  /// its top edge in the viewport. Null when no plain row is on screen.
  ({String key, double y, double globalY})? plainTop() {
    final viewport = plainViewport.currentContext?.findRenderObject();
    if (viewport is! RenderBox || !viewport.hasSize) return null;
    int? best;
    double? bestY;
    for (final entry in plainRows.entries) {
      if (layout == null || entry.key >= layout!.rows.length) continue;
      final row = entry.value.currentContext?.findRenderObject();
      if (row is! RenderBox || !row.attached || !row.hasSize) continue;
      final y = row.localToGlobal(Offset.zero, ancestor: viewport).dy;
      if (y + row.size.height <= 0 || y >= viewport.size.height) continue;
      if (bestY == null || y < bestY) {
        bestY = y;
        best = entry.key;
      }
    }
    if (best == null || bestY == null) return null;
    final item = layout!.rows[best];
    if (item is! ItemRow) return null;
    return (
      key:
          'item:${layout!.firstMessageFor(item.item.id) ?? item.item.id.split('#').first}',
      y: bestY,
      globalY: viewport.localToGlobal(Offset.zero).dy + bestY,
    );
  }

  void scheduleReaderMeasure() {
    if (_readerMeasureScheduled) return;
    _readerMeasureScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _readerMeasureScheduled = false;
      if (!mounted()) return;
      readingAnchor = _messageAtTop({
        for (final message in store().state?.messages ?? const <AgentMessage>[])
          message.id,
      });
    });
  }

  /// Only mounted message blocks are measured; lazy, unseen rows stay unbuilt.
  /// Anchoring inside a grouped turn also handles a new section splitting it.
  ({String id, double y})? _messageAtTop(Set<String> ids) {
    final root = transcriptViewport.currentContext;
    final viewport = root?.findRenderObject();
    if (root == null || viewport is! RenderBox || !viewport.hasSize) {
      return null;
    }
    final top = viewport.localToGlobal(Offset.zero).dy;
    final bottom = top + viewport.size.height;
    ({String id, double y})? anchor;
    void visit(Element element) {
      final key = element.widget.key;
      if (key is ValueKey<String> && key.value.startsWith('agent-msg-')) {
        final id = key.value.substring('agent-msg-'.length);
        final box = ids.contains(id) ? element.findRenderObject() : null;
        if (box is RenderBox && box.attached && box.hasSize) {
          final y = box.localToGlobal(Offset.zero).dy;
          if (y < bottom &&
              y + box.size.height > top &&
              (anchor == null || y < anchor!.y)) {
            anchor = (id: id, y: y);
          }
        }
      }
      element.visitChildElements(visit);
    }

    root.visitChildElements(visit);
    return anchor;
  }

  double? _messageY(String id) {
    double? y;
    void visit(Element element) {
      if (element.widget.key == ValueKey('agent-msg-$id')) {
        final box = element.findRenderObject();
        if (box is RenderBox && box.attached && box.hasSize) {
          y = box.localToGlobal(Offset.zero).dy;
        }
        return;
      }
      element.visitChildElements(visit);
    }

    transcriptViewport.currentContext?.visitChildElements(visit);
    return y;
  }

  void keepMessagePosition(
    ({String id, double y}) anchor,
    ChatOutline raw, {
    bool afterInitialLayout = false,
  }) {
    final token = ++positionRestore;
    final session = stateFor;
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      // The newly mounted indexed list restores its own origin this frame.
      // Measure after that layout, including the header's actual height.
      if (afterInitialLayout) await WidgetsBinding.instance.endOfFrame;
      bool current() =>
          mounted() &&
          token == positionRestore &&
          session == stateFor &&
          identical(raw, rawOutline) &&
          store().pendingFor(presentation()) == null;
      if (!current()) return;
      var y = _messageY(anchor.id);
      if (y == null) {
        final target = layout?.target(anchor.id, anchor.id);
        final index = target == null
            ? null
            : layout?.rowIndex(OutlineLayout.itemKey(target.itemId));
        if (index == null) return;
        list.jumpToIndex(index);
        await WidgetsBinding.instance.endOfFrame;
        if (!current()) return;
        y = _messageY(anchor.id);
      }
      if (y == null || !list.scroll.hasClients) return;
      final delta = y - anchor.y;
      if (delta.abs() < .5) return;
      final position = list.scroll.position;
      list.scroll.jumpTo(
        (position.pixels + delta).clamp(
          position.minScrollExtent,
          position.maxScrollExtent,
        ),
      );
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }
}
