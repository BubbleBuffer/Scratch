import 'navigation.dart';
import '../../services/addon_services.dart';

import 'package:flutter/widgets.dart';

import '../../src/rust/api/agent.dart' show ChatOutline;
import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/anchored_list.dart';
import '../../ui/ui.dart';
import 'add_to_note.dart';
import 'agent_cells.dart';
import 'agent_composer.dart';
import 'agent_markdown.dart';
import 'agent_outline.dart';
import 'agent_outline_view.dart';
import 'agent_store.dart';
import 'agent_transcript.dart';
import 'agent_turn.dart';
import 'chat_reference.dart';
import 'dev_replay.dart';

const agentEmptyLine =
    'Ask the agent to find, summarize, write or reorganize pages in this vault. Its edits show in page history like your own.';

/// The same transcript and composer serve the Agent section and editor dock.
class AgentChat extends StatefulWidget {
  const AgentChat({
    super.key,
    required this.store,
    this.panel = false,
    this.title,
  });
  final AgentStore store;
  final bool panel;

  /// The full-screen host's header. The dock supplies its own header.
  final String? title;
  @override
  State<AgentChat> createState() => _AgentChatState();
}

class _AgentChatState extends State<AgentChat> with TickerProviderStateMixin {
  /// The outlined transcript, laid out by row index with message anchors.
  final _list = AnchoredListController();

  /// The plain list: a chat with no outline and no pending reference. It keeps
  /// its scroll controller and its spacing until the first outline arrives.
  final _scroll = ScrollController();

  /// Chapters whose messages the reader has shown, keyed by the chapter's first
  /// message id, which outlives the chapter's id. Explicit states only: false
  /// is an explicit fold, true an explicit unfold. Others follow the default.
  final _shown = <String, bool>{};
  final _expanded = <String, bool>{};

  /// Chapters that closed while the reader was inside them. They stay open,
  /// under a quiet "Chapter closed · Fold" heading, until the reader scrolls
  /// away or taps Fold. Keyed by first message id.
  final _deferred = <String>{};

  /// Chapters whose messages are collapsing, by first message id. Each runs
  /// its own animation from open to closed and hides the messages at its end.
  final _collapsing = <String, AnimationController>{};

  /// The chapter, by first message id, of each item row laid out so far, so a
  /// folded item can rebase on its chapter's card.
  final _itemChapter = <String, String>{};

  /// Whether the rows are laid out by index. Latches on: once an outline or a
  /// pending reference arrives, the chat stays on the indexed list.
  bool _indexed = false;

  /// The plain list's rows by index, and its viewport, so the top row can be
  /// measured when the chat switches to the indexed list.
  final _plainRows = <int, GlobalKey>{};
  final _plainViewport = GlobalKey();
  final _transcriptViewport = GlobalKey();
  ChatOutline? _rawOutline;
  int _positionRestore = 0;
  ({String id, double y})? _readingAnchor;
  bool _readerMeasureScheduled = false;

  /// Where the indexed list starts after the switch: the outline row of the
  /// plain list's top row, and that row's top edge in the viewport.
  ({int index, double y, double globalY})? _restore;

  static const _collapseDuration = Duration(milliseconds: 240);
  static const _full = AlwaysStoppedAnimation<double>(1);

  /// First message ids of the chapters seen open, so a close is noticed.
  final _openFirsts = <String>{};

  /// The chat the fold and place state belong to. Another chat resets it.
  String? _stateFor;

  final _where = ValueNotifier<OutlineWhere>((
    chapter: null,
    section: null,
    chapterId: null,
    sectionId: null,
  ));

  /// The row index at the top of the transcript, as the list reports it, and
  /// the first and last visible rows.
  int? _top;
  (int, int)? _visibleRange;

  /// Whether the reader is at the end of the transcript. Only then does a new
  /// reply follow the end; a reader who sent a message always goes to it.
  bool get _atEnd =>
      !_list.scroll.hasClients || _list.scroll.position.extentAfter < 24;
  int _lastMessages = 0;
  String _last = '';

  /// The outline and layout of the last build. Jumps and the strip read them.
  OutlineView _outline = OutlineView.empty;
  OutlineLayout? _layout;

  /// The stable key of each row of the last build, and its index. The list
  /// rebases on these, so a row keeps its place across outline changes.
  List<String> _stableKeys = const [];
  Map<String, int> _stableIndex = const {};
  Set<String> _flashKeys = {};
  int _flashToken = 0;
  bool _taking = false;

  @override
  void initState() {
    super.initState();
    widget.store.showSession(true);
    widget.store.ensureOpen();
  }

  @override
  void didUpdateWidget(AgentChat oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.store != widget.store) {
      oldWidget.store.showSession(false);
      widget.store.showSession(true);
    }
  }

  @override
  void dispose() {
    widget.store.showSession(false);
    for (final c in _collapsing.values) {
      c.dispose();
    }
    _list.dispose();
    _scroll.dispose();
    _where.dispose();
    super.dispose();
  }

  /// A closed chapter's messages are hidden until shown. An open chapter's
  /// are always shown.
  bool _hidden(ChapterView chapter) =>
      chapter.foldedByDefault && !(_shown[chapter.first] ?? false);

  void _setShown(ChapterView chapter, bool shown) {
    setState(() {
      _cancelCollapse(chapter.first);
      _shown[chapter.first] = shown;
    });
  }

  /// Notices the chapters that closed since the last build. One that closed
  /// while the reader is inside it stays open under its "Chapter closed · Fold"
  /// heading. Any other is shown for one frame and then folds, so its card
  /// animates the hide.
  void _closeFolds(OutlineView outline) {
    final inView = _chaptersInView();
    // A live update can extend the chapter and close it together. Classify
    // the visible message against its new anchors as well as the old rows.
    final messages = widget.store.state?.messages ?? const <AgentMessage>[];
    // The anchor is measured after layout; walking children during build is
    // unsafe while the lazy transcript is replacing its rows.
    final top = _readingAnchor;
    final position = messages.indexWhere((m) => m.id == top?.id);
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
        _openFirsts.add(c.first);
        continue;
      }
      if (!_openFirsts.remove(c.first)) continue;
      _shown[c.first] = true;
      if (inView.contains(c.first)) {
        _deferred.add(c.first);
      } else {
        _foldAfterFrame(c.first);
      }
    }
  }

  /// Folds [first]'s messages on the next frame, once the card has shown them.
  void _foldAfterFrame(String first) {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || _deferred.contains(first) || _shown[first] != true) {
        return;
      }
      _collapse(first);
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  /// Collapses [first]'s message rows over [_collapseDuration], then hides
  /// them. The list is first pinned to the row at the top of the viewport, so
  /// rows above it shrinking cannot move what the reader is looking at.
  void _collapse(String first) {
    if (_collapsing.containsKey(first)) return;
    _list.pinTop();
    final animation = AnimationController(
      vsync: this,
      duration: _collapseDuration,
    );
    setState(() => _collapsing[first] = animation);
    animation.reverse(from: 1).whenComplete(() {
      if (!mounted || !identical(_collapsing[first], animation)) return;
      setState(() {
        _collapsing.remove(first)?.dispose();
        _shown.remove(first);
      });
    });
  }

  /// Stops a collapse in progress when the reader shows or folds the chapter
  /// by hand.
  void _cancelCollapse(String first) => _collapsing.remove(first)?.dispose();

  /// Ends the deferred state of [first] and folds its messages.
  void _foldNow(String first) {
    setState(() => _deferred.remove(first));
    _foldAfterFrame(first);
  }

  /// The first message ids of the chapters with a row on screen, in the last
  /// build's layout.
  Set<String> _chaptersInView() {
    final layout = _layout, range = _visibleRange;
    if (layout == null || range == null || layout.rows.isEmpty) return {};
    final last = range.$2 < layout.rows.length
        ? range.$2
        : layout.rows.length - 1;
    final out = <String>{};
    for (var i = range.$1; i <= last; i++) {
      final ch = layout.rows[i].chapter;
      if (ch >= 0 && ch < _outline.chapters.length) {
        out.add(_outline.chapters[ch].first);
      }
    }
    return out;
  }

  /// Folds the deferred chapters the reader has scrolled away from.
  void _foldAwayFromDeferred() {
    if (_deferred.isEmpty) return;
    final inView = _chaptersInView();
    for (final first in _deferred.toList()) {
      if (!inView.contains(first)) _foldNow(first);
    }
  }

  static String _chapterKey(ChapterView c) => 'chap:first:${c.first}';

  static String _sectionKey(SectionView s) => 'sec:first:${s.first}';

  /// Stable row keys use the first message carried by each turn piece, so a
  /// section inserted earlier cannot change a later piece's identity.
  List<String> _stableKeysOf(OutlineLayout layout, OutlineView outline) {
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
          _itemChapter[key] = row.chapter >= 0
              ? outline.chapters[row.chapter].first
              : '';
          keys.add(key);
        case ChapterRow():
          keys.add(_chapterKey(outline.chapters[row.chapter]));
        case SectionRow():
          keys.add(
            _sectionKey(outline.chapters[row.chapter].sections[row.section]),
          );
      }
    }
    return keys;
  }

  /// The row index of a stable key. An item that is folded away maps to its
  /// chapter's card, so the list rebases there rather than losing its place.
  int? _indexOfKey(String key) {
    final direct = _stableIndex[key];
    if (direct != null || !key.startsWith('item:')) return direct;
    final first = _itemChapter[key];
    if (first == null || first.isEmpty) return null;
    return _stableIndex['chap:first:$first'];
  }

  Future<void> _toggleMessages(ChapterView chapter, bool shown) async {
    _positionRestore++;
    _setShown(chapter, shown);
    await WidgetsBinding.instance.endOfFrame;
    if (!mounted) return;
    final index = _layout?.rowIndex('chap:${chapter.id}');
    if (index != null) _list.jumpToIndex(index);
  }

  /// Where this chat is shown. A reference names the presentation it is for.
  AgentPresentation get _presentation =>
      widget.panel ? AgentPresentation.dock : AgentPresentation.screen;

  /// Follows the end when a reply or a message arrives. The outline's own
  /// changes do not count, and a reader away from the end keeps their place
  /// unless they sent the message.
  void _follow(AgentStore s) {
    final messages = s.state?.messages ?? const <AgentMessage>[];
    final token =
        '${s.openId}:${messages.length}:${s.streaming}:${s.state?.edits.length}:${s.state?.activity.length}';
    if (_last == token) return;
    final sent =
        messages.length > _lastMessages &&
        messages.lastOrNull?.role == 'user' &&
        !(s.replay?.active == true && s.replay?.sessionId == s.openId);
    _last = token;
    _lastMessages = messages.length;
    if (!sent && !(_indexed ? _atEnd : _plainAtEnd)) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      // A pending reference jumps instead of following the end.
      if (!mounted || widget.store.pendingFor(_presentation) != null) return;
      if (_indexed) {
        _list.jumpToEnd();
      } else if (_scroll.hasClients) {
        _scroll.jumpTo(_scroll.position.maxScrollExtent);
      }
    });
  }

  /// Shows a pending reference once its chat and outline have both loaded.
  void _maybeTakePending() {
    if (widget.store.pendingFor(_presentation) == null || _taking) return;
    _taking = true;
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      try {
        if (!mounted) return;
        final s = widget.store;
        final p = s.pendingFor(_presentation);
        if (p == null ||
            s.state?.session.id != p.sessionId ||
            s.outlineFor != p.sessionId) {
          return;
        }
        s.takePendingAnchor(_presentation);
        await _jump(p.firstMessageId, p.lastMessageId);
      } finally {
        _taking = false;
      }
    });
  }

  /// Shows the span from [first] to [last]. A hidden chapter is shown first,
  /// then the list jumps by row index to the span's first shown row (the
  /// section's divider when [sectionKey] names one) and flashes the span.
  Future<void> _jump(String first, String last, {String? sectionKey}) async {
    _positionRestore++;
    var layout = _layout;
    var target = layout?.target(first, last);
    if (layout == null || target == null) return;
    if (target.chapter >= 0 && _hidden(_outline.chapters[target.chapter])) {
      _setShown(_outline.chapters[target.chapter], true);
      await WidgetsBinding.instance.endOfFrame;
      if (!mounted) return;
      layout = _layout;
      target = layout?.target(first, last);
      if (layout == null || target == null) return;
    }
    final index =
        (sectionKey == null ? null : layout.rowIndex(sectionKey)) ??
        layout.rowIndex(OutlineLayout.itemKey(target.itemId));
    if (index == null) return;
    setState(() {
      _flashKeys = layout!.spanKeys(first, last);
      _flashToken++;
    });
    _list.jumpToIndex(index);
  }

  /// Called by the list when the visible rows change: reads the chapter and
  /// section at the top into the where-am-I strip, tracks whether the reader
  /// is at the end, and folds deferred chapters the reader has left.
  void _onVisible(int first, int last) {
    _top = first;
    _visibleRange = (first, last);
    _refreshWhere();
    _foldAwayFromDeferred();
  }

  void _refreshWhere() {
    final layout = _layout;
    final top = _top;
    final OutlineRow? row =
        layout != null && top != null && top < layout.rows.length
        ? layout.rows[top]
        : null;
    final chapter = row == null || row.chapter < 0
        ? null
        : _outline.chapters[row.chapter];
    final section = chapter == null || row!.section < 0
        ? null
        : chapter.sections[row.section];
    final next = (
      chapter: chapter?.title,
      section: section?.title,
      chapterId: chapter?.id,
      sectionId: section?.id,
    );
    if (_where.value != next) _where.value = next;
  }

  /// Opens the contents sheet; a tap there reveals the chapter or section.
  void _openContents() {
    showOutlineContents(
      context,
      outline: _outline,
      where: _where.value,
      onChapter: _openChapter,
      onSection: (chapter, section) =>
          _jump(section.first, section.last, sectionKey: 'sec:${section.id}'),
    );
  }

  /// Shows a chapter from the contents: a closed chapter's messages are shown
  /// under its card, and the list jumps to the card or heading.
  Future<void> _openChapter(ChapterView chapter) async {
    _positionRestore++;
    final key = 'chap:${chapter.id}';
    if (_hidden(chapter)) {
      _setShown(chapter, true);
      await WidgetsBinding.instance.endOfFrame;
      if (!mounted) return;
    }
    final index = _layout?.rowIndex(key);
    if (index == null) return;
    setState(() {
      _flashKeys = {key};
      _flashToken++;
    });
    _list.jumpToIndex(index);
  }

  void _chapterMenu(ChapterView chapter) {
    showOutlineMenu(
      context,
      actions: [('Add to note…', Ico.plus, () => _addToNote(chapter, null))],
    );
  }

  void _sectionMenu(ChapterView chapter, SectionView section) {
    showOutlineMenu(
      context,
      actions: [('Add to note…', Ico.plus, () => _addToNote(chapter, section))],
    );
  }

  /// Adds the chapter, or its section, to a note through the reference sheet.
  /// The outline keeps its own view types, so the raw chapter from the store
  /// is found by id.
  void _addToNote(ChapterView chapter, SectionView? section) {
    final store = widget.store;
    final sessionId = store.openId;
    final raw = store.outline.chapters
        .where((c) => c.id == chapter.id)
        .firstOrNull;
    if (sessionId == null || raw == null) return;
    final title = store.displaySessionTitle(
      store.openId ?? '',
      store.state?.session.title ?? '',
    );
    if (section == null) {
      showAddChatReference(
        context,
        ChatReference.fromChapter(
          sessionId: sessionId,
          chatTitle: title,
          chapter: raw,
        ),
      );
      return;
    }
    final rawSection = raw.sections
        .where((s) => s.id == section.id)
        .firstOrNull;
    if (rawSection == null) return;
    showAddChatReference(
      context,
      ChatReference.fromSection(
        sessionId: sessionId,
        chatTitle: title,
        chapter: raw,
        section: rawSection,
      ),
    );
  }

  /// A row of the plain list: one chat item, spaced as the plain list always
  /// has been. Keyed by index, so the top row can be measured on screen.
  Widget _plainRow(int i, ChatItem item) => Center(
    key: _plainRows.putIfAbsent(i, () => GlobalKey()),
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: Layout.contentWidth),
      child: Padding(
        padding: const EdgeInsets.only(bottom: 14),
        child: _itemView(item),
      ),
    ),
  );

  /// The plain list's top row, as an index of the outline layout and the y of
  /// its top edge in the viewport. Null when no plain row is on screen.
  ({String key, double y, double globalY})? _plainTop() {
    final viewport = _plainViewport.currentContext?.findRenderObject();
    if (viewport is! RenderBox || !viewport.hasSize) return null;
    int? best;
    double? bestY;
    for (final entry in _plainRows.entries) {
      if (_layout == null || entry.key >= _layout!.rows.length) continue;
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
    final item = _layout!.rows[best];
    if (item is! ItemRow) return null;
    return (
      key:
          'item:${_layout!.firstMessageFor(item.item.id) ?? item.item.id.split('#').first}',
      y: bestY,
      globalY: viewport.localToGlobal(Offset.zero).dy + bestY,
    );
  }

  /// Whether the plain list shows its last row, or is not yet laid out.
  bool get _plainAtEnd =>
      !_scroll.hasClients || _scroll.position.extentAfter < 24;

  void _scheduleReaderMeasure() {
    if (_readerMeasureScheduled) return;
    _readerMeasureScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _readerMeasureScheduled = false;
      if (!mounted) return;
      _readingAnchor = _messageAtTop({
        for (final message
            in widget.store.state?.messages ?? const <AgentMessage>[])
          message.id,
      });
    });
  }

  /// Only mounted message blocks are measured; lazy, unseen rows stay unbuilt.
  /// Anchoring inside a grouped turn also handles a new section splitting it.
  ({String id, double y})? _messageAtTop(Set<String> ids) {
    final root = _transcriptViewport.currentContext;
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

    _transcriptViewport.currentContext?.visitChildElements(visit);
    return y;
  }

  void _keepMessagePosition(
    ({String id, double y}) anchor,
    ChatOutline raw, {
    bool afterInitialLayout = false,
  }) {
    final token = ++_positionRestore;
    final session = _stateFor;
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      // The newly mounted indexed list restores its own origin this frame.
      // Measure after that layout, including the header's actual height.
      if (afterInitialLayout) await WidgetsBinding.instance.endOfFrame;
      bool current() =>
          mounted &&
          token == _positionRestore &&
          session == _stateFor &&
          identical(raw, _rawOutline) &&
          widget.store.pendingFor(_presentation) == null;
      if (!current()) return;
      var y = _messageY(anchor.id);
      if (y == null) {
        final target = _layout?.target(anchor.id, anchor.id);
        final index = target == null
            ? null
            : _layout?.rowIndex(OutlineLayout.itemKey(target.itemId));
        if (index == null) return;
        _list.jumpToIndex(index);
        await WidgetsBinding.instance.endOfFrame;
        if (!current()) return;
        y = _messageY(anchor.id);
      }
      if (y == null || !_list.scroll.hasClients) return;
      final delta = y - anchor.y;
      if (delta.abs() < .5) return;
      final position = _list.scroll.position;
      _list.scroll.jumpTo(
        (position.pixels + delta).clamp(
          position.minScrollExtent,
          position.maxScrollExtent,
        ),
      );
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  /// The row widget for [row]: the chat item, a chapter's card or heading, or
  /// a section divider.
  Widget _rowView(OutlineRow row) {
    final Widget child = switch (row) {
      ItemRow(:final item) => _itemView(item),
      ChapterRow(:final folded) => _chapterRow(
        _outline.chapters[row.chapter],
        row.chapter,
        folded,
      ),
      SectionRow() => OutlineSectionDivider(
        key: ValueKey('outline-section-${row.key}'),
        section: _outline.chapters[row.chapter].sections[row.section],
        color: outlineColor(context, row.section + 1),
        onMenu: () => _sectionMenu(
          _outline.chapters[row.chapter],
          _outline.chapters[row.chapter].sections[row.section],
        ),
      ),
    };
    final view = Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: Layout.contentWidth),
        child: Padding(
          padding: const EdgeInsets.only(bottom: 14),
          child: _flashKeys.contains(row.key)
              ? OutlineFlash(token: _flashToken, child: child)
              : child,
        ),
      ),
    );
    if (row is! ItemRow) return view;
    // A collapsing chapter's messages shrink to nothing. Every item row keeps
    // this wrapper, at full size when not collapsing, so the row's subtree does
    // not change shape when a collapse starts.
    final chapter = row.chapter >= 0 && row.chapter < _outline.chapters.length
        ? _outline.chapters[row.chapter].first
        : null;
    final Animation<double> collapse =
        (chapter == null ? null : _collapsing[chapter]) ?? _full;
    return SizeTransition(
      sizeFactor: collapse,
      alignment: Alignment.topCenter,
      child: view,
    );
  }

  Widget _chapterRow(ChapterView chapter, int index, bool folded) {
    final color = outlineColor(context, index);
    if (chapter.ongoing) {
      return OutlineChapterHeading(
        key: ValueKey('outline-chapter-${chapter.id}'),
        chapter: chapter,
        color: color,
        onMenu: () => _chapterMenu(chapter),
        closed: false,
        onFold: null,
      );
    }
    if (_deferred.contains(chapter.first)) {
      return OutlineChapterHeading(
        key: ValueKey('outline-chapter-${chapter.id}'),
        chapter: chapter,
        color: color,
        onMenu: () => _chapterMenu(chapter),
        closed: true,
        onFold: () => _foldNow(chapter.first),
      );
    }
    final shown = !folded && !_collapsing.containsKey(chapter.first);
    return OutlineChapterCard(
      key: ValueKey('outline-chapter-${chapter.id}'),
      chapter: chapter,
      color: color,
      messagesShown: shown,
      onToggleMessages: () => _toggleMessages(chapter, !shown),
      onAddToNote: () => _addToNote(chapter, null),
      onOpenSection: (section) =>
          _jump(section.first, section.last, sectionKey: 'sec:${section.id}'),
      open: _expanded[chapter.first] ?? false,
      onOpenChanged: (open) => setState(() => _expanded[chapter.first] = open),
    );
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.store,
    builder: (context, _) {
      final store = widget.store;
      final c = context.colors;
      if (_stateFor != store.openId) {
        _stateFor = store.openId;
        _shown.clear();
        _expanded.clear();
        _deferred.clear();
        _openFirsts.clear();
        _itemChapter.clear();
        _indexed = false;
        _restore = null;
        _plainRows.clear();
        for (final c in _collapsing.values) {
          c.dispose();
        }
        _collapsing.clear();
        _flashKeys = {};
        _top = null;
        _visibleRange = null;
        _rawOutline = null;
        _positionRestore++;
        _readingAnchor = null;
        _lastMessages = 0;
        _last = '';
      }
      final messages = store.state?.messages ?? const <AgentMessage>[];
      final outline = store.outline.chapters.isEmpty
          ? OutlineView.empty
          : OutlineView.of(store.outline, messages: messages);
      final changed = !identical(_rawOutline, store.outline);
      final sent =
          messages.length > _lastMessages &&
          messages.lastOrNull?.role == 'user' &&
          !(store.replay?.active == true &&
              store.replay?.sessionId == store.openId);
      final keep =
          changed &&
              !sent &&
              !(_indexed ? _atEnd : _plainAtEnd) &&
              store.pendingFor(_presentation) == null
          ? _readingAnchor
          : null;
      _rawOutline = store.outline;
      final plainTop = !_indexed && !outline.isEmpty ? _plainTop() : null;
      if (plainTop != null && !_plainAtEnd) {
        // An already-closed chapter arriving around the reader stays open.
        final item = _layout!.rows
            .whereType<ItemRow>()
            .where(
              (r) =>
                  'item:${_layout!.firstMessageFor(r.item.id) ?? r.item.id.split('#').first}' ==
                  plainTop.key,
            )
            .firstOrNull;
        final msg =
            _readingAnchor?.id ??
            (item == null ? null : _layout!.firstMessageFor(item.item.id));
        final position = msg == null
            ? -1
            : messages.indexWhere((m) => m.id == msg);
        for (final chapter in outline.chapters) {
          final a = messages.indexWhere((m) => m.id == chapter.first);
          final b = messages.indexWhere((m) => m.id == chapter.last);
          if (!chapter.ongoing && a >= 0 && a <= position && b >= position) {
            _shown[chapter.first] = true;
            _deferred.add(chapter.first);
          }
        }
      }
      _closeFolds(outline);
      _outline = outline;
      final messageIds = [for (final m in messages) m.id];
      final segments = outlineSegments(outline, messageIds);
      final anchors = <String, String>{};
      final items = agentTurns(
        store.state,
        streaming: store.streaming,
        error: store.error,
        approvals: store.pendingApprovals,
        anchorOf: anchors,
        segmentOf: segments.isEmpty ? null : segments,
      );
      final layout = OutlineLayout.build(
        outline: outline,
        items: items,
        messageIds: messageIds,
        anchorOf: anchors,
        folded: _hidden,
      );
      _layout = layout;
      _stableKeys = _stableKeysOf(layout, outline);
      _stableIndex = {
        for (var i = 0; i < _stableKeys.length; i++) _stableKeys[i]: i,
      };
      final wasIndexed = _indexed;
      _indexed =
          _indexed ||
          !outline.isEmpty ||
          store.pendingFor(_presentation) != null;
      if (_indexed && !wasIndexed && plainTop != null) {
        final index = _indexOfKey(plainTop.key);
        if (index != null) {
          _restore = (index: index, y: plainTop.y, globalY: plainTop.globalY);
        }
      }
      _follow(store);
      if (keep != null && _indexed) {
        _keepMessagePosition(
          keep,
          store.outline,
          afterInitialLayout: !wasIndexed,
        );
      }
      _scheduleReaderMeasure();
      if (!outline.isEmpty) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) _refreshWhere();
        });
      }
      _maybeTakePending();
      // Android stays in Hub mode; signed-out chat offers browser sign-in.
      final signIn = store.requiresHubSignIn;
      final notice = signIn
          ? null
          : !store.enabled
          ? 'The agent is off. Turn it on in Settings > AI.'
          : store.openError;
      final pad = widget.panel ? 12.0 : 32.0;
      final padding = EdgeInsets.fromLTRB(pad, widget.panel ? 14 : 28, pad, 16);
      final Widget body = signIn
          ? Center(
              child: Padding(
                padding: EdgeInsets.all(pad),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 460),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(
                        'Sign in to your notes hub to chat with the agent. It reads and edits this vault there.',
                        key: const ValueKey('agent-sign-in-prompt'),
                        textAlign: TextAlign.center,
                        style: ts(context.type.body, c.textSecondary),
                      ),
                      const SizedBox(height: 16),
                      PrimaryButton(
                        key: const ValueKey('agent-sign-in'),
                        label: 'Sign in',
                        onTap: () =>
                            showSettings(context, section: SettingsSection.sync),
                      ),
                    ],
                  ),
                ),
              ),
            )
          : notice != null
          ? Center(
              child: Padding(
                padding: EdgeInsets.all(pad),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 460),
                  child: Text(
                    notice,
                    key: const ValueKey('agent-open-error'),
                    style: ts(context.type.body, c.textSecondary),
                  ),
                ),
              ),
            )
          : items.isEmpty
          ? Center(
              child: Padding(
                padding: EdgeInsets.all(pad),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 460),
                  child: Text(
                    agentEmptyLine,
                    key: const ValueKey('agent-empty'),
                    textAlign: TextAlign.center,
                    style: ts(context.type.body, c.textSecondary),
                  ),
                ),
              ),
            )
          : _indexed
          ? AnchoredList(
              controller: _list,
              itemCount: layout.rows.length,
              padding: padding,
              onVisibleRangeChanged: _onVisible,
              keyOf: (i) => i < _stableKeys.length ? _stableKeys[i] : null,
              indexOf: _indexOfKey,
              initialIndex: _restore?.index,
              initialY: _restore?.y ?? 0,
              initialGlobalY: _restore?.globalY,
              itemBuilder: (context, i) => _rowView(layout.rows[i]),
            )
          : ListView.builder(
              key: _plainViewport,
              controller: _scroll,
              padding: padding,
              itemCount: items.length,
              itemBuilder: (context, i) => _plainRow(i, items[i]),
            );
      return Container(
        decoration: BoxDecoration(
          color: c.bgPrimary,
          borderRadius: Radii.panel,
          boxShadow: widget.panel ? null : c.seam,
        ),
        child: ClipRRect(
          borderRadius: Radii.panel,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (!widget.panel && widget.title != null)
                Padding(
                  padding: const EdgeInsets.fromLTRB(24, 12, 24, 10),
                  child: Text(
                    widget.title!,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: tsDisplay(context.type.h3, c.textPrimary),
                  ),
                ),
              if (outline.isEmpty &&
                  !widget.panel &&
                  store.hostInfo?.kind != 'hub' &&
                  (store.hostInfo?.projects ?? false) &&
                  store.state?.session.projectId != null)
                Padding(
                  padding: const EdgeInsets.fromLTRB(24, 12, 24, 0),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: SelectButton<String>(
                      key: const ValueKey('agent-session-project'),
                      title: 'Project',
                      value: store.state!.session.projectId!,
                      options: [for (final p in store.projects) p.id],
                      labelOf: (id) =>
                          store.projects
                              .where((p) => p.id == id)
                              .firstOrNull
                              ?.name ??
                          store.state?.session.projectName ??
                          'Project',
                      onChanged: (id) =>
                          store.setSessionProject(store.state!.session.id, id),
                    ),
                  ),
                ),
              if (!outline.isEmpty)
                OutlineWhereStrip(
                  where: _where,
                  onTap: _openContents,
                  colors: {
                    for (var i = 0; i < outline.chapters.length; i++)
                      outline.chapters[i].id: outlineColor(context, i),
                  },
                ),
              Expanded(
                child: NotificationListener<ScrollNotification>(
                  onNotification: (_) {
                    _scheduleReaderMeasure();
                    return false;
                  },
                  child: SizedBox.expand(key: _transcriptViewport, child: body),
                ),
              ),
              if (store.replay case final playback?
                  when playback.sessionId == store.openId)
                ReplayControls(playback: playback),
              if (!signIn)
                Center(
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(
                      maxWidth: Layout.contentWidth + 64,
                    ),
                    child: Padding(
                      padding: EdgeInsets.fromLTRB(
                        pad,
                        0,
                        pad,
                        widget.panel ? 12 : 20,
                      ),
                      child: AgentComposer(store: store, panel: widget.panel),
                    ),
                  ),
                ),
            ],
          ),
        ),
      );
    },
  );

  Widget _itemView(ChatItem item) => switch (item) {
    UserText() => _UserBubble(
      message: item,
      sessionId: widget.store.openId,
      onOpenChat: widget.store.openSession,
      chatTitle: widget.store.displaySessionTitle,
      key: ValueKey('agent-msg-${item.id}'),
    ),
    AgentTurn() => AgentTurnView(
      turn: item,
      store: widget.store,
      key: ValueKey('agent-turn-${item.id}'),
    ),
    AssistantText() => AgentMarkdown(
      item.text,
      key: ValueKey('agent-msg-${item.id}'),
    ),
    CodeCell() => CodeCellRow(cell: item, key: ValueKey('agent-${item.id}')),
    RunNote() => Text(
      item.text,
      key: ValueKey('agent-note-${item.id}'),
      style: ts(
        context.type.uiSmall,
        item.error
            ? context.colors.inlineCodeText
            : context.colors.textTertiary,
      ),
    ),
    ApprovalItem() => ApprovalCard(
      approval: item.approval,
      key: ValueKey('agent-approval-${item.approval.id}'),
      onDecide: (decision, reason) =>
          widget.store.decide(item.approval.id, decision, reason: reason),
    ),
  };
}

class _UserBubble extends StatelessWidget {
  const _UserBubble({
    super.key,
    required this.message,
    required this.onOpenChat,
    required this.chatTitle,
    this.sessionId,
  });
  final UserText message;
  final String? sessionId;
  final ValueChanged<String> onOpenChat;
  final String Function(String id, String raw) chatTitle;
  Widget _chip(
    BuildContext context,
    String label,
    Key key,
    Ico icon,
    VoidCallback onTap, {
    required double availableWidth,
  }) {
    final c = context.colors;
    return Hover(
      key: key,
      onTap: onTap,
      radius: 14,
      border: Border.all(color: c.borderSubtle),
      background: c.bgPrimary,
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon20(icon, size: 12, color: c.iconTertiary),
          const SizedBox(width: 5),
          ConstrainedBox(
            constraints: BoxConstraints(
              maxWidth: (availableWidth - 35).clamp(0.0, 200.0).toDouble(),
            ),
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(context.type.uiSmall, c.textSecondary),
            ),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final selection = message.context?.selection;
    final pages = context.watch<PagesStore>();
    return Align(
      alignment: Alignment.centerRight,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 560),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 9),
          decoration: BoxDecoration(
            color: c.bgSecondary,
            borderRadius: Radii.panel,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              if ((message.context?.pages.isNotEmpty ?? false) ||
                  (message.context?.chats.isNotEmpty ?? false) ||
                  selection != null) ...[
                LayoutBuilder(
                  builder: (context, bounds) => Wrap(
                    spacing: 4,
                    runSpacing: 4,
                    children: [
                      for (final page in message.context?.pages ?? [])
                        _chip(
                          context,
                          displayTitle(
                            page.title,
                            firstLine: pages.meta(page.id)?.firstLine ?? '',
                          ),
                          ValueKey('agent-msg-${message.id}-page-${page.id}'),
                          Ico.page,
                          () => openAgentPage(
                            context,
                            page.id,
                            page.title,
                            sessionId: sessionId,
                          ),
                          availableWidth: bounds.maxWidth,
                        ),
                      for (final chat in message.context?.chats ?? [])
                        _chip(
                          context,
                          chatTitle(chat.id, chat.title),
                          ValueKey('agent-msg-${message.id}-chat-${chat.id}'),
                          Ico.comment,
                          () => onOpenChat(chat.id),
                          availableWidth: bounds.maxWidth,
                        ),
                      if (selection != null)
                        _chip(
                          context,
                          'Selection',
                          ValueKey('agent-msg-${message.id}-selection'),
                          Ico.text,
                          () => openAgentPage(
                            context,
                            selection.pageId,
                            selection.pageTitle,
                            blocks: selection.blockIds,
                            sessionId: sessionId,
                          ),
                          availableWidth: bounds.maxWidth,
                        ),
                    ],
                  ),
                ),
                const SizedBox(height: 6),
              ],
              Text(message.text, style: ts(context.type.body, c.textPrimary)),
            ],
          ),
        ),
      ),
    );
  }
}
