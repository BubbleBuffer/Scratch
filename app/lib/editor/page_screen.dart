// The page editor: cover, icon and title, then the page's blocks in one
// lazy list (the big page has 2,000). Each block is its own EditableText;
// this state owns the document model, the per-block controllers and focus
// nodes, and every editing command (split, merge, indent, `/` and `@`
// menus, marks, moves, multi-block selection, clipboard, undo). What each
// block kind does comes from the block registry. Edits go to the core
// through the page's [PageSession] (see state/page_store.dart), which also
// hands back changes made elsewhere and keeps the page's undo stack.

import '../core/debounce.dart';

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../core/page_link.dart';
import '../blocks/heading.dart';
import '../shell/page_stack.dart';
import '../addons/agent/api.dart';
import '../features/ai/ask_ai.dart';
import '../features/bakeoff.dart';
import '../features/backlinks/backlinks.dart';
import '../addons/database/api.dart' show PageProperties;
import '../registry/note_type_registry.dart';
import '../registry/addon_registry.dart' show AddonInlineMark, NoteTitleChips;
import '../registry/block_codecs.dart';
import '../registry/embed_block.dart';
import '../shell/addons.dart';
import '../shell/addon_panels.dart';
import '../features/history/history_panel.dart';
import '../features/import/import_panel.dart';
import '../features/page/breadcrumbs.dart';
import '../features/page/page_header.dart';
import '../features/page/page_menu.dart';
import '../features/templates/templates.dart';
import '../features/sync/conflicts.dart';
import '../features/sync/sync_indicator.dart';
import '../features/share/share_popup.dart';
import '../state/sync_store.dart';
import '../features/trash/trash_panel.dart';
import '../registry/block_registry.dart';
import '../shell/move_to.dart';
import '../shell/navigation.dart';
import '../shell/mobile_page_history.dart';
import '../shell/sidebar.dart';
import '../state/app_state.dart';
import '../state/editor_focus.dart';
import '../state/page_store.dart';
import '../state/pages_store.dart';
import '../state/undo.dart';
import '../ui/emoji_picker.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import '../ui/calico_art.dart';
import 'block_pickers.dart';
import 'block_row.dart';
import 'clipboard.dart';
import 'color_menu.dart';
import 'mention_menu.dart';
import 'menus.dart';
import 'model.dart';
import 'rich_controller.dart';
import 'todo_dates.dart';

class PageScreen extends StatefulWidget {
  const PageScreen({
    super.key,
    required this.pageId,
    required this.initialTitle,
    this.isNew = false,
    this.initialBlock,
    this.snapshot,
    this.navigationPreview = false,
    this.embedded = false,
    this.initialEditingValue,
  });
  final String pageId;
  final String initialTitle;
  final bool isNew;
  final String? initialBlock;

  /// A past version to show read-only (version history): the real blocks,
  /// nothing saved, no outside changes adopted; no sidebar and no top bar.
  final PageData? snapshot;

  /// An inert sheet revealed underneath a mobile history drag.
  final bool navigationPreview;

  /// Reuses this editor inside an enclosing, vertically scrolling workspace.
  /// The caller owns page headings/navigation; blocks and editing stay here.
  final bool embedded;

  /// Transfers a bootstrap input's caret/composition to [initialBlock].
  final TextEditingValue? initialEditingValue;

  @override
  State<PageScreen> createState() => PageEditorState();
}

class PageEditorState extends State<PageScreen>
    implements BlockHost, HeadingProgressHost, HeadingCompletionHost {
  Doc? doc;
  // The focusable first line of an empty page stays local until a block edit.
  // Its insert is sent before that edit, in the same transaction.
  List<BOp>? _emptyParagraph;
  int _sheetIndex = 1;
  double _sheetLeft = PileGeometry.margin;
  PageStackNavigator? _sheetNavigator;
  late final ValueChanged<String> _navigationReveal = _revealNavigationBlock;
  late final ValueChanged<List<String>> _navigationHighlight = highlightBlocks;
  late final EditorFocus _editorFocus;
  bool _focusUpdatePending = false;
  double? _sheetWidth;
  @override
  List<Node> get headingProgressOrder => doc?.flat ?? const [];
  @override
  String? get headingProgressSpaceColor {
    final ancestors = _meta.ancestors(widget.pageId);
    return ancestors.isEmpty ? _page?.color : ancestors.first.color;
  }

  final Map<String, int> _headingCompletions = {};
  @override
  int headingCompletionRevision(String id) => _headingCompletions[id] ?? 0;

  bool get _emptyPage =>
      doc != null &&
      doc!.flat.every(
        (n) =>
            n.kind == Kinds.paragraph && n.text.isEmpty && n.children.isEmpty,
      );

  late final PageStore _pages;
  late final PagesStore _meta;
  String? Function(String)? _embedTitles;
  late final PageSession session;
  List<FlatBlock> _readCurrentBlocks() => [
    for (final b in doc?.toFlat() ?? <FlatBlock>[])
      if (b.id != _emptyParagraph?.single.id) b,
  ];
  String _readCurrentTitle() => titleCtl.text;
  late final List<FlatBlock> Function() _currentBlocks = _readCurrentBlocks;
  late final String Function() _currentTitle = _readCurrentTitle;
  final _embeddedOverlay = OverlayPortalController();
  bool _embeddedOverlayUpdatePending = false;
  ScrollPosition? _embeddedParentScroll;
  bool _resumeBootstrapInput = false;
  late final titleCtl = TextEditingController(text: widget.initialTitle);
  late final titleFocus = FocusNode(onKeyEvent: _onTitleKey);
  final scroll = ScrollController();
  final stackKey = GlobalKey();
  final listKey = GlobalKey();
  double _loadMs = 0;
  bool _reportedOpen = false;
  bool _scrolled = false;
  Timer? _renameTimer;
  Timer? _blockHighlightTimer;
  final Set<String> highlightedBlocks = {};
  // Retained for callers revealing a single navigation target.
  String? get highlightedBlock => highlightedBlocks.firstOrNull;
  List<String> _pendingHighlightBlocks = const [];
  String? _revealBlock;
  int _revealTries = 0;
  double _revealLow = 0;
  double? _revealHigh;

  final _ctl = <String, RichController>{};
  final _fn = <String, FocusNode>{};
  final _keys = <String, GlobalKey<EditableTextState>>{};
  final rows = <String, BlockRowState>{};

  String? focusedId;

  /// Selected blocks (Esc, drag-select, shift-click; select mode on mobile).
  final Set<String> selected = {};
  String? _selAnchor;
  late final _pageFocus = FocusNode(
    onKeyEvent: (_, e) => _onPageKey(e),
    debugLabel: 'page',
  );

  /// Mobile select mode (from the block sheet): taps toggle selection.
  bool selectMode = false;

  // `/` menu
  String? slashId;
  String slashQuery = '';
  int slashIndex = 0;
  bool slashTurnInto = false;
  Rect? slashAnchor;

  // `@` menu
  String? mentionId;
  int mentionStart = 0;

  /// The menu was opened by `[[` (pages only) rather than `@`.
  bool mentionPagesOnly = false;
  String get _mentionTrigger => mentionPagesOnly ? '[[' : '@';
  String mentionQuery = '';
  int mentionIndex = 0;
  List<MentionItem> mentionItems = const [];
  Rect? mentionAnchor;
  int _mentionGen = 0;

  // selection toolbar (desktop), link box
  Rect? selRect;
  bool _pointerDown = false;
  (String, TextSelection)? linkTarget;
  Rect? linkAnchor;
  final Map<BMarkKind, bool> _pending = {};

  // block menu (desktop)
  String? menuBlockId;
  Rect? menuAnchor;

  // drag (desktop)
  Node? dragNode;
  int? dropGap;
  double? dropY, dropX;
  Offset? dragPos;

  /// A drop beside a top-level block (making columns): the block, which
  /// side, and the line shown there (in the stack).
  Node? dropBeside;
  bool dropRight = false;
  Rect? dropSideLine;

  // drag-select (desktop)
  Offset? _marqueeStart, _marqueeEnd;
  bool _marquee = false;

  KbMode kbMode = KbMode.main;
  bool _softBreak = false;
  String? _pendingFocus;
  int _focusTries = 0;

  /// This page's open toggles, remembered on this device across runs.
  late final Set<String> _open = context.app.ui.openToggles(widget.pageId);

  PagePrefs get prefs => _meta.prefs(widget.pageId);
  bool get readOnly => preview || prefs.locked;

  /// Showing [PageScreen.snapshot].
  bool get preview => widget.snapshot != null;
  bool get smallText => prefs.smallText;
  bool get _canAskAgent => AgentStore.of(context.app).enabled;

  // ---------------------------------------------------------------------
  // Lifecycle.

  PageData? _page;

  @override
  void initState() {
    super.initState();
    _editorFocus = EditorFocus.forApp(context.app);
    titleCtl.addListener(_scheduleEditorFocus);
    if (!preview && !widget.embedded) Bakeoff.instance.screen = 'page';
    _pages = context.read<PageStore>();
    _meta = context.read<PagesStore>();
    final vault = context.app.vault;
    final metadata = _meta;
    _embedTitles = (id) => metadata.meta(id)?.title ?? embedTitleOf(vault, id);
    addEmbedTitleResolver(_embedTitles!);
    _meta.addListener(_onMeta);
    if (!widget.navigationPreview) {
      session = preview
          ? _pages.detached(widget.pageId)
          : _pages.open(widget.pageId);
      session.queue.onError = (e) => debugPrint('apply failed: $e');
      session.currentBlocks = _currentBlocks;
      session.currentTitle = _currentTitle;
      session.external.addListener(_adoptExternal);
      session.deleted.addListener(_onDeleted);
      session.undo.addListener(_onUndoChanged);
    }
    _load();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (preview) return;
    if (widget.embedded) {
      _sheetNavigator = PageStackScope.maybeOf(context);
      final position = Scrollable.maybeOf(context)?.position;
      if (!identical(position, _embeddedParentScroll)) {
        _embeddedParentScroll?.removeListener(_embeddedScrollChanged);
        _embeddedParentScroll = position;
        position?.addListener(_embeddedScrollChanged);
      }
      _scheduleEditorFocus();
      return;
    }
    final stack = PageStackScope.of(context);
    if (!identical(_sheetNavigator, stack)) {
      _unregisterNavigationReveal();
      _sheetNavigator = stack;
      stack.revealBlocks[widget.pageId] = _navigationReveal;
      stack.highlightBlocks[widget.pageId] = _navigationHighlight;
      stack.deliverHighlights(widget.pageId);
      _reportFirstLine();
    }
    _scheduleEditorFocus();
  }

  void _unregisterNavigationReveal() {
    final stack = _sheetNavigator;
    if (stack != null &&
        identical(stack.revealBlocks[widget.pageId], _navigationReveal)) {
      stack.revealBlocks.remove(widget.pageId);
    }
    if (stack != null &&
        identical(stack.highlightBlocks[widget.pageId], _navigationHighlight)) {
      stack.highlightBlocks.remove(widget.pageId);
    }
  }

  bool get _isTopEditor {
    if (preview || _saved) return false;
    if (widget.embedded) {
      return _pageFocus.hasFocus &&
          (ModalRoute.of(context)?.isCurrent ?? true) &&
          _sheetNavigator?.topId == null;
    }
    final stack = _sheetNavigator;
    if (stack?.topId != widget.pageId) return false;
    final current = stack?.currentRoute;
    // Popup menus belong to their page; another screen does not.
    return current is PopupRoute || identical(current, ModalRoute.of(context));
  }

  void _scheduleEditorFocus() {
    if (_focusUpdatePending || !mounted || preview || _saved) return;
    _focusUpdatePending = true;
    SchedulerBinding.instance.addPostFrameCallback((_) {
      _focusUpdatePending = false;
      if (mounted && !_saved) _publishEditorFocus();
    });
  }

  void _publishEditorFocus() {
    if (!_isTopEditor) {
      // A full-page editor of the same document can now own this selection.
      if (!widget.embedded || _sheetNavigator?.topId != widget.pageId) {
        _editorFocus.clear(widget.pageId);
      }
      return;
    }
    final d = doc;
    final blocks = d == null
        ? <Node>[]
        : d.flat.where((n) => selected.contains(n.id)).toList();
    var ids = blocks.map((n) => n.id).toList();
    var text = blocks.map((n) => _ctl[n.id]?.text ?? n.text).join('\n');
    if (blocks.isEmpty) {
      final n = focusedNode;
      final controller = n == null ? null : _ctl[n.id];
      final selection = controller?.selection;
      if (controller != null &&
          selection != null &&
          selection.isValid &&
          !selection.isCollapsed) {
        ids = [n!.id];
        text = selection.textInside(controller.text);
      }
    }
    _editorFocus.update(
      pageId: widget.pageId,
      pageTitle: titleCtl.text,
      blockIds: ids,
      text: text,
    );
  }

  /// Capture the exact editor selection before opening the agent composer.
  void askAgentForSelection({Node? block}) {
    if (!_isTopEditor) return;
    _publishEditorFocus();
    if (block != null && selected.isEmpty) {
      _editorFocus.update(
        pageId: widget.pageId,
        pageTitle: titleCtl.text,
        blockIds: [block.id],
        text: _ctl[block.id]?.text ?? block.text,
      );
    }
    setState(() => menuBlockId = null);
    unawaited(askAgent(context));
  }

  /// Ask AI about the selected text, the selected blocks or [block]; the
  /// answer goes after the last of them.
  void askAiForSelection({Node? block}) {
    final d = doc;
    if (d == null || readOnly) return;
    var blocks = d.flat.where((n) => selected.contains(n.id)).toList();
    if (blocks.isEmpty && block != null) blocks = [block];
    String text;
    if (blocks.isNotEmpty) {
      text = blocks.map((n) => _ctl[n.id]?.text ?? n.text).join('\n');
    } else {
      final n = focusedNode;
      if (n == null) return;
      final controller = _ctl[n.id];
      final selection = controller?.selection;
      blocks = [n];
      text = controller != null && selection != null && selection.isValid && !selection.isCollapsed
          ? selection.textInside(controller.text)
          : controller?.text ?? n.text;
    }
    setState(() {
      menuBlockId = null;
      if (!selectMode) selected.clear();
    });
    unawaited(_askAi(after: blocks.last.id, selection: text));
  }

  /// The Ask AI box about this page (saved first, so the run reads it as
  /// shown); the answer goes after block [after] or at the page's end.
  Future<void> _askAi({String? after, String? selection}) async {
    _saveTitle();
    await session.save();
    if (!mounted) return;
    await showAskAi(context, pageId: widget.pageId, after: after, selection: selection);
  }

  /// A notification can target a retained sheet, including a hidden to-do.
  void _revealNavigationBlock(String id) {
    highlightBlocks([id]);
  }

  /// Flash every referenced block and scroll the first one into view.
  void highlightBlocks(List<String> ids) {
    if (!mounted) return;
    _pendingHighlightBlocks = List.of(ids);
    _blockHighlightTimer?.cancel();
    setState(highlightedBlocks.clear);
    if (ids.isEmpty) {
      _revealBlock = null;
      return;
    }
    _revealBlock = ids.first;
    _revealTries = 0;
    _revealLow = 0;
    _revealHigh = null;
    final d = doc;
    if (d == null) return; // Loading applies the pending navigation target.
    final targets = [
      for (final id in ids)
        if (d.byId[id] != null) d.byId[id]!,
    ];
    if (targets.isEmpty) {
      _revealBlock = null;
      return;
    }
    _revealBlock = targets.first.id;
    for (final target in targets) {
      for (var parent = target.parent; parent != null; parent = parent.parent) {
        _open.add(parent.id);
      }
    }
    d.reflatten();
    setState(() {});
    // The highlight signals successful reveal, including for retained lazy
    // sheets. Its three-second lifetime starts once the first row is visible.
    SchedulerBinding.instance.addPostFrameCallback(
      (_) => _revealInitialBlock(),
    );
  }

  void _onMeta() => _safeSetState();
  void _onUndoChanged() => _safeSetState();

  Future<void> _load() async {
    final sw = Stopwatch()..start();
    final PageData p;
    try {
      p = widget.snapshot ?? await session.load();
    } catch (e) {
      debugPrint('load failed: $e');
      return;
    }
    _loadMs = p.loadUs / 1000.0;
    if (!preview) _meta.adopt(p);
    final d = Doc.fromPage(p);
    d.isOpen = isOpen;
    d.reflatten();
    if (!mounted) return;
    _ensureEmptyParagraph(d);
    if (titleCtl.text != p.title) titleCtl.text = p.title;
    setState(() {
      doc = d;
      _page = p;
    });
    _reportFirstLine();
    if (_pendingHighlightBlocks.isNotEmpty) {
      highlightBlocks(_pendingHighlightBlocks);
    } else if (widget.initialBlock != null) {
      _revealNavigationBlock(widget.initialBlock!);
    }
    debugPrint('page loaded in ${sw.elapsedMilliseconds} ms');
    final initialValue = widget.initialEditingValue;
    final initialNode = d.byId[widget.initialBlock];
    if (initialValue != null && initialNode != null) {
      final controller = ctl(initialNode);
      controller.value = initialValue;
      _resumeBootstrapInput = true;
      focusBlock(initialNode, initialValue.selection.extentOffset);
      // focusBlock collapses the caret; restore the bootstrap selection/IME.
      controller.value = initialValue;
    } else if (widget.isNew && !widget.embedded) {
      SchedulerBinding.instance.addPostFrameCallback(
        (_) => titleFocus.requestFocus(),
      );
    }
  }

  void _ensureEmptyParagraph(Doc d) {
    if (d.flat.isNotEmpty || preview) {
      _emptyParagraph = null;
      return;
    }
    // Reuse the local id on an external metadata change so focus and the
    // controller survive. It is excluded from the session's persisted view.
    final n = Node(
      id: _emptyParagraph?.single.id ?? newBlockId(),
      kind: Kinds.paragraph,
    );
    _emptyParagraph = d.insertFirstChild(null, n);
  }

  /// The page changed outside this editor (sync, restore, another window):
  /// adopt the new blocks, keeping controllers, focus and carets.
  void _adoptExternal() {
    final p = session.external.value;
    final old = doc;
    if (p == null || old == null || !mounted) return;
    _meta.adopt(p);
    final d = Doc.fromPage(p);
    d.isOpen = isOpen;
    d.reflatten();
    if (_emptyParagraph != null) _ensureEmptyParagraph(d);
    for (final n in d.flat) {
      final c = _ctl[n.id];
      if (c == null) continue;
      if (c.text != n.text || !_sameMarks(c.marks, n.marks)) {
        final sel = c.selection;
        c.marks = List.of(n.marks);
        c.value = TextEditingValue(
          text: n.text,
          selection: sel.isValid
              ? TextSelection(
                  baseOffset: sel.baseOffset.clamp(0, n.text.length),
                  extentOffset: sel.extentOffset.clamp(0, n.text.length),
                )
              : TextSelection.collapsed(offset: n.text.length),
        );
      }
    }
    for (final id in old.byId.keys.toList()) {
      if (!d.byId.containsKey(id)) _forget(id);
    }
    if (!titleFocus.hasFocus && titleCtl.text != p.title) {
      titleCtl.text = p.title;
    }
    d.title = titleCtl.text;
    if (slashId != null && !d.byId.containsKey(slashId)) slashId = null;
    if (mentionId != null && !d.byId.containsKey(mentionId)) mentionId = null;
    if (menuBlockId != null && !d.byId.containsKey(menuBlockId)) {
      menuBlockId = null;
    }
    selected.removeWhere((id) => !d.byId.containsKey(id));
    setState(() {
      doc = d;
      _page = p;
    });
    _reportFirstLine();
  }

  /// The first line of body text, which lists show for an untitled page.
  static String firstLineOf(Doc d) =>
      firstLineOfTexts(d.flat.map((n) => n.text));

  void _reportFirstLine() {
    if (widget.embedded) return;
    final d = doc;
    if (d != null) _sheetNavigator?.setFirstLine(widget.pageId, firstLineOf(d));
  }

  static bool _sameMarks(List<BMark> a, List<BMark> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }

  void _onDeleted() {
    if (!session.deleted.value || !mounted) return;
    if (widget.embedded) return; // Its owner reloads the day-note lookup.
    final route = ModalRoute.of(context);
    if (route != null && route.isCurrent) Navigator.of(context).maybePop();
  }

  bool _saved = false;

  /// Menus and an editor selection own their gesture until dismissed.
  bool get navigationBlocked =>
      selectMode ||
      selected.isNotEmpty ||
      slashId != null ||
      mentionId != null ||
      menuBlockId != null ||
      linkTarget != null ||
      dragNode != null ||
      _marquee ||
      (titleFocus.hasFocus && !titleCtl.selection.isCollapsed) ||
      _ctl.entries.any(
        (entry) =>
            (_fn[entry.key]?.hasFocus ?? false) &&
            !entry.value.selection.isCollapsed,
      );

  /// Capture the live document without opening another editing session.
  PageData? navigationSnapshot() {
    final p = _page, d = doc;
    if (p == null || d == null) return null;
    return PageData(
      id: p.id,
      title: titleCtl.text,
      blocks: d.toFlat(),
      parent: p.parent,
      icon: p.icon,
      cover: p.cover,
      color: p.color,
      role: p.role,
      properties: p.properties,
      pageType: p.pageType,
      favorite: p.favorite,
      inTrash: p.inTrash,
      createdMs: p.createdMs,
      updatedMs: p.updatedMs,
      prefs: p.prefs,
      loadUs: p.loadUs,
    );
  }

  /// Called only when a history move commits, never for a cancelled drag.
  void leaveMobileHistory() => _saveNow();

  /// Writes the title and hands the session back (the store saves it); runs
  /// when the page is popped (before the list reloads) and again, as a
  /// no-op, on dispose.
  void _saveNow() {
    if (_saved) return;
    _saved = true;
    if (widget.navigationPreview) return;
    if (!widget.embedded || _sheetNavigator?.topId != widget.pageId) {
      _editorFocus.clear(widget.pageId);
    }
    _unregisterNavigationReveal();
    _renameTimer?.cancel();
    final d = doc;
    if (d != null &&
        !preview &&
        titleCtl.text != d.title &&
        !session.deleted.value) {
      d.title = titleCtl.text;
      session.rename(titleCtl.text);
    }
    session.external.removeListener(_adoptExternal);
    session.deleted.removeListener(_onDeleted);
    session.undo.removeListener(_onUndoChanged);
    if (identical(session.currentBlocks, _currentBlocks)) {
      session.currentBlocks = null;
    }
    if (identical(session.currentTitle, _currentTitle)) {
      session.currentTitle = null;
    }
    _pages.close(session);
  }

  /// Moves this page to the trash; the session's `deleted` pops the editor.
  Future<void> moveToTrash() async {
    _renameTimer?.cancel();
    final d = doc;
    if (d != null && titleCtl.text != d.title) {
      d.title = titleCtl.text;
      session.rename(titleCtl.text);
    }
    await _pages.trash(widget.pageId);
  }

  @override
  void dispose() {
    _embeddedParentScroll?.removeListener(_embeddedScrollChanged);
    if (_embedTitles != null) removeEmbedTitleResolver(_embedTitles!);
    _blockHighlightTimer?.cancel();
    if (!preview && !widget.embedded) Bakeoff.instance.screen = 'list';
    _saveNow();
    _meta.removeListener(_onMeta);
    for (final c in _ctl.values) {
      c.dispose();
    }
    for (final f in _fn.values) {
      f.dispose();
    }
    titleCtl.dispose();
    titleFocus.dispose();
    _pageFocus.dispose();
    scroll.dispose();
    super.dispose();
  }

  // ---------------------------------------------------------------------
  // Per-block objects, created lazily as rows are built.

  RichController ctl(Node n) =>
      _ctl[n.id] ??= RichController(text: n.text, marks: List.of(n.marks))
        ..colors = AppTheme.of(context).colors
        ..addListener(_scheduleEditorFocus);

  /// The controller of [n], if one exists. Nothing is made here.
  RichController? controllerOf(Node n) => _ctl[n.id];

  /// The add-ons' inline marks: the chips the editor draws and opens.
  Iterable<AddonInlineMark> get inlineMarks => appAddonContext.inlineMarks.all;
  FocusNode fn(Node n) => _fn[n.id] ??= FocusNode(
    onKeyEvent: (_, e) => onKey(n, e),
    debugLabel: n.id,
  );
  GlobalKey<EditableTextState> editableKey(Node n) =>
      _keys[n.id] ??= GlobalKey<EditableTextState>();

  void register(BlockRowState r) => rows[r.node.id] = r;
  void unregister(String id, BlockRowState r) {
    if (rows[id] == r) rows.remove(id);
  }

  void _forget(String id) {
    _ctl.remove(id)?.dispose();
    final f = _fn.remove(id);
    if (f != null) {
      SchedulerBinding.instance.addPostFrameCallback((_) => f.dispose());
    }
    _keys.remove(id);
    selected.remove(id);
  }

  Node? get focusedNode => focusedId == null ? null : doc?.byId[focusedId];

  void _safeSetState([VoidCallback? fn]) {
    if (!mounted) return;
    if (SchedulerBinding.instance.schedulerPhase ==
        SchedulerPhase.persistentCallbacks) {
      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (mounted) setState(fn ?? () {});
      });
    } else {
      setState(fn ?? () {});
    }
  }

  void _push(List<BOp>? ops) {
    if (ops == null || ops.isEmpty || preview) return;
    final first = _emptyParagraph;
    _emptyParagraph = null;
    session.apply([...?first, ...ops]);
    _reportFirstLine();
  }

  // ---------------------------------------------------------------------
  // Undo and redo. A command runs inside [rec]: the document is
  // snapshotted before and after, and the difference (both ways) becomes
  // one entry. Typing in a block is one entry per run (a pause of a second
  // or a move to another block starts a new one).

  List<SnapBlock>? _snapBefore;
  Caret? _caretBefore;
  String _recLabel = '';
  int _recDepth = 0;
  UndoEntry? _typingEntry;
  final _typingClock = Stopwatch();

  T rec<T>(String label, T Function() f) {
    final d = doc;
    if (d == null) return f();
    if (_recDepth++ == 0) {
      _snapBefore = d.snapshot();
      _caretBefore = _caret();
      _recLabel = label;
    }
    try {
      return f();
    } finally {
      if (--_recDepth == 0) _commitRec();
    }
  }

  void _commitRec() {
    final d = doc, before = _snapBefore;
    _snapBefore = null;
    if (d == null || before == null) return;
    final after = d.snapshot();
    final redo = diffSnapshots(before, after);
    if (redo.isEmpty) return;
    _typingEntry = null;
    session.undo.push(
      UndoEntry(
        label: _recLabel,
        undo: diffSnapshots(after, before),
        redo: redo,
        before: _caretBefore,
        after: _caret(),
      ),
    );
  }

  Caret? _caret() {
    final id = _pendingFocus ?? focusedId;
    if (id == null) return null;
    final off = _ctl[id]?.selection.baseOffset ?? 0;
    return (id, math.max(0, off));
  }

  void _recordTyping(Node n, String oldText, List<BMark> oldMarks, int caret) {
    if (_recDepth > 0) return;
    final redo = n.setTextOp();
    final top = session.undo.last;
    final oldCaret = math.max(0, caret - (n.text.length - oldText.length));
    if (top != null &&
        identical(top, _typingEntry) &&
        top.before?.$1 == n.id &&
        _typingClock.elapsedMilliseconds < 1000) {
      top.redo = [redo];
      top.after = (n.id, caret);
    } else {
      final e = UndoEntry(
        label: 'Typing',
        undo: [
          BOp(
            op: OpKind.setText,
            id: n.id,
            kind: n.kind,
            text: oldText,
            marks: List.of(oldMarks),
            checked: n.checked,
          ),
        ],
        redo: [redo],
        before: (n.id, oldCaret),
        after: (n.id, caret),
      );
      session.undo.push(e);
      _typingEntry = e;
    }
    _typingClock
      ..reset()
      ..start();
  }

  bool get canUndo => !widget.navigationPreview && session.undo.canUndo;
  bool get canRedo => !widget.navigationPreview && session.undo.canRedo;

  void undo() {
    if (readOnly) return;
    final e = session.undo.popUndo();
    if (e != null) _applyHistory(e.undo, e.before);
  }

  void redo() {
    if (readOnly) return;
    final e = session.undo.popRedo();
    if (e != null) _applyHistory(e.redo, e.after);
  }

  void _applyHistory(List<BOp> ops, Caret? caret) {
    final d = doc;
    if (d == null) return;
    _typingEntry = null;
    d.applyOps(ops);
    session.apply(ops);
    for (final id in _ctl.keys.toList()) {
      final n = d.byId[id];
      if (n == null) {
        _forget(id);
        continue;
      }
      final c = _ctl[id]!;
      if (c.text != n.text || !_sameMarks(c.marks, n.marks)) {
        c.marks = List.of(n.marks);
        c.value = TextEditingValue(
          text: n.text,
          selection: TextSelection.collapsed(offset: n.text.length),
        );
      }
    }
    setState(() {
      selected.clear();
      slashId = null;
      mentionId = null;
      menuBlockId = null;
    });
    final n = caret == null ? null : d.byId[caret.$1];
    if (n != null && blockRegistry.of(n.kind).editable) {
      for (final p in d.closedAncestors(n)) {
        _open.add(p.id);
      }
      d.reflatten();
      focusBlock(n, caret!.$2);
    } else {
      FocusManager.instance.primaryFocus?.unfocus();
      _pageFocus.requestFocus();
    }
  }

  // ---------------------------------------------------------------------
  // Focus.

  void focusBlock(Node n, int offset) {
    final c = ctl(n);
    c.selection = TextSelection.collapsed(
      offset: offset.clamp(0, c.text.length),
    );
    _pendingFocus = n.id;
    _focusTries = 0;
    _safeSetState();
    SchedulerBinding.instance.addPostFrameCallback((_) => _applyPendingFocus());
  }

  void _applyPendingFocus() {
    final id = _pendingFocus;
    final d = doc;
    if (id == null || d == null || !mounted) return;
    final n = d.byId[id];
    if (n == null) {
      _pendingFocus = null;
      return;
    }
    final r = rows[id];
    if (r != null && r.mounted) {
      _pendingFocus = null;
      fn(n).requestFocus();
      _ensureVisible(r);
      if (_resumeBootstrapInput && widget.initialBlock == id) {
        _resumeBootstrapInput = false;
        // The bootstrap input already wrote this text. Resume ordinary menu
        // parsing once its real block row/caret is laid out and focused.
        _updateSlash(n, ctl(n).value);
        _updateMention(n, ctl(n).value, typed: true);
      }
      return;
    }
    if (++_focusTries > 6 || !scroll.hasClients) return;
    // Not built yet: jump near it, then try again next frame.
    final top = d.embedder(n) ?? n;
    final est = 140.0 + math.max(0, top.visIndex) * 30.0;
    scroll.jumpTo(est.clamp(0, scroll.position.maxScrollExtent));
    SchedulerBinding.instance.addPostFrameCallback((_) => _applyPendingFocus());
    SchedulerBinding.instance.scheduleFrame();
  }

  void _ensureVisible(BlockRowState r) {
    if (widget.embedded) {
      unawaited(Scrollable.ensureVisible(r.context));
      return;
    }
    final ro = r.context.findRenderObject();
    if (ro == null || !scroll.hasClients) return;
    final vp = RenderAbstractViewport.maybeOf(ro);
    if (vp == null) return;
    final top = vp.getOffsetToReveal(ro, 0).offset;
    final bottom = vp.getOffsetToReveal(ro, 1).offset;
    final cur = scroll.offset;
    if (cur > top) {
      scroll.jumpTo(top.clamp(0, scroll.position.maxScrollExtent));
    } else if (cur < bottom) {
      scroll.jumpTo(bottom.clamp(0, scroll.position.maxScrollExtent));
    }
  }

  void _revealInitialBlock() {
    final id = _revealBlock;
    final d = doc;
    if (!mounted || id == null || d == null || !scroll.hasClients) return;
    final row = rows[id];
    if (row != null && row.mounted) {
      _revealBlock = null;
      _ensureVisible(row);
      setState(() {
        highlightedBlocks
          ..clear()
          ..addAll(_pendingHighlightBlocks.where(d.byId.containsKey));
      });
      _blockHighlightTimer?.cancel();
      _blockHighlightTimer = Timer(const Duration(seconds: 3), () {
        if (mounted) setState(highlightedBlocks.clear);
      });
      return;
    }
    final n = d.byId[id];
    if (n == null || ++_revealTries > 24) {
      _revealBlock = null;
      return;
    }
    final top = d.embedder(n) ?? n;
    // A lazy page may have tall wrapped paragraphs, headings and media.
    // Re-estimate from the rows actually laid out, then narrow the known
    // offset bounds; repeating a fixed height estimate cannot find them.
    final laidOut = <({int index, double start, double end})>[];
    for (final entry in rows.entries) {
      final node = d.byId[entry.key];
      if (node == null || node.visIndex < 0 || !entry.value.mounted) continue;
      final box = entry.value.context.findRenderObject();
      if (box is! RenderBox || !box.attached || !box.hasSize) continue;
      final viewport = RenderAbstractViewport.maybeOf(box);
      if (viewport == null) continue;
      final start = viewport.getOffsetToReveal(box, 0).offset;
      laidOut.add((
        index: node.visIndex,
        start: start,
        end: start + box.size.height,
      ));
    }
    laidOut.sort((a, b) => a.index.compareTo(b.index));
    var estimate = 140.0 + math.max(0, top.visIndex) * 30.0;
    if (laidOut.isNotEmpty) {
      final first = laidOut.first, last = laidOut.last;
      final meanHeight = math.max(
        1.0,
        (last.end - first.start) / (last.index - first.index + 1),
      );
      estimate = first.start + (top.visIndex - first.index) * meanHeight;
      if (top.visIndex > last.index) {
        _revealLow = math.max(_revealLow, last.end);
      }
      if (top.visIndex < first.index) {
        _revealHigh = math.min(_revealHigh ?? first.start, first.start);
      }
      final high = _revealHigh;
      if (high != null &&
          high > _revealLow &&
          (estimate <= _revealLow ||
              estimate >= high ||
              (estimate - scroll.offset).abs() < 1)) {
        estimate = (_revealLow + high) / 2;
      } else {
        estimate = math.max(_revealLow, estimate);
      }
    }
    scroll.jumpTo(estimate.clamp(0, scroll.position.maxScrollExtent));
    SchedulerBinding.instance.addPostFrameCallback(
      (_) => _revealInitialBlock(),
    );
    SchedulerBinding.instance.scheduleFrame();
  }

  void onFocus(Node n, bool has) {
    if (has) {
      focusedId = n.id;
      if (!selectMode) selected.clear();
      if (menuBlockId != null) menuBlockId = null;
      Bakeoff.instance.log('focus kind=${n.kind} index=${n.flatIndex}');
      _safeSetState();
    } else if (focusedId == n.id) {
      focusedId = null;
      if (slashId == n.id) slashId = null;
      if (mentionId == n.id) mentionId = null;
      selRect = null;
      _pending.clear();
      _safeSetState();
    }
  }

  // ---------------------------------------------------------------------
  // BlockHost.

  @override
  String get pageId => widget.pageId;

  @override
  bool isOpen(Node n) => _open.contains(n.id);

  @override
  void toggleOpen(Node n) {
    final d = doc;
    if (d == null) return;
    if (!_open.remove(n.id)) _open.add(n.id);
    d.reflatten();
    final f = focusedNode;
    if (f != null && d.closedAncestors(f).isNotEmpty) {
      // The caret was inside: it moves to the toggle.
      focusBlock(n, n.text.length);
    }
    setState(() {});
  }

  @override
  void addChild(Node n) {
    if (readOnly) return;
    final p = Node(id: newBlockId(), kind: Kinds.paragraph);
    rec('Add block', () {
      _open.add(n.id);
      _push(doc!.insertFirstChild(n, p));
    });
    setState(() {});
    focusBlock(p, 0);
  }

  @override
  void setTarget(Node n, String? target) {
    // Async callers (the inline database block) may land after the page closed.
    if (!mounted || readOnly) return;
    rec('Edit block', () => _push(doc!.setTarget(n, target)));
    setState(() {});
  }

  @override
  void setWidth(Node n, int? width) {
    if (readOnly) return;
    rec('Resize', () => _push(doc!.setWidth(n, width)));
    setState(() {});
  }

  @override
  void setText(Node n, String text) {
    if (readOnly) return;
    rec('Edit text', () {
      n.text = text;
      n.marks = [];
      final c = ctl(n);
      c.marks = n.marks;
      c.value = TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      );
      _push([n.setTextOp()]);
    });
    setState(() {});
  }

  @override
  void replaceWithBlock(Node paragraph, String type, Map<String, Object?> data) {
    if (readOnly) return;
    rec('Turn into', () {
      paragraph.marks = [];
      paragraph.text = _dataText(data);
      _push([...doc!.setKind(paragraph, type), paragraph.setTextOp()]);
      _syncController(paragraph);
    });
    setState(() {});
  }

  @override
  void insertBlockAfter(
    Node paragraph,
    String type,
    Map<String, Object?> data, {
    TextRange? removeText,
  }) {
    if (readOnly) return;
    rec('Add block', () {
      final ops = <BOp>[];
      final cut = removeText;
      if (cut != null && !cut.isCollapsed) {
        paragraph.marks = _cutMarks(paragraph.marks, cut);
        paragraph.text = paragraph.text.replaceRange(cut.start, cut.end, '');
        ops.add(paragraph.setTextOp());
        _syncController(paragraph);
      }
      final block = Node(id: newBlockId(), kind: type, text: _dataText(data));
      ops.addAll(doc!.insertBelow(paragraph, block));
      _push(ops);
    });
    setState(() {});
  }

  @override
  void replaceWithParagraph(Node block, String text, List<BMark> marks) {
    if (readOnly) return;
    rec('Turn into', () {
      block.marks = List.of(marks);
      block.text = text;
      _push([...doc!.setKind(block, Kinds.paragraph), block.setTextOp()]);
      _syncController(block);
    });
    setState(() {});
  }

  /// The `text` field of a block's [data] (see [BlockHost.replaceWithBlock]).
  String _dataText(Map<String, Object?> data) => (data['text'] as String?) ?? '';

  /// [marks] after [cut] is removed from the text: marks inside the cut are
  /// dropped, marks after it move left, and marks across it are clipped.
  List<BMark> _cutMarks(List<BMark> marks, TextRange cut) {
    final len = cut.end - cut.start;
    int map(int x) => x <= cut.start ? x : (x >= cut.end ? x - len : cut.start);
    return [
      for (final m in marks)
        if (map(m.end) > map(m.start))
          BMark(start: map(m.start), end: map(m.end), kind: m.kind, url: m.url),
    ];
  }

  /// Brings [n]'s controller in line with the model after a structural edit.
  void _syncController(Node n) {
    final c = _ctl[n.id];
    if (c == null) return;
    c.marks = List.of(n.marks);
    c.value = TextEditingValue(
      text: n.text,
      selection: TextSelection.collapsed(offset: n.text.length),
    );
  }

  @override
  Widget nestedBlocks(Node n, double width, double gutter) {
    final d = doc;
    if (d == null) return const SizedBox.shrink();
    final list = d.rowsUnder(n);
    final geo = Geo(gutter, width);
    if (list.isEmpty) {
      return GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => addChild(n),
        child: const SizedBox(height: 28),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        for (final c in list)
          BlockRow(
            key: ValueKey(c.id),
            node: c,
            ed: this,
            geo: geo,
            baseDepth: n.depth + 1,
          ),
      ],
    );
  }

  /// The conflict review on this page's blocks (features/sync/conflicts.dart).
  /// None in a historical preview: it shows the page as it was, with nothing to act on.
  ConflictBlocks? _conflictBlocks;
  BlockDecor? get blockDecor {
    if (preview || widget.navigationPreview) return null;
    final d = _conflictBlocks;
    if (d != null && d.pageId == widget.pageId) return d;
    return _conflictBlocks = ConflictBlocks(sync: context.read<SyncStore>(), pageId: widget.pageId);
  }

  @override
  void openPage(String id) {
    final m = _meta.meta(id);
    _saveTitle();
    navOpenPage(context, id, m?.title ?? '', stack: true);
  }

  @override
  List<Node> get readingOrder => doc?.reading ?? const [];

  @override
  void revealBlock(Node n) {
    final d = doc;
    if (d == null) return;
    for (final p in d.closedAncestors(n)) {
      _open.add(p.id);
    }
    d.reflatten();
    setState(() {});
    if (blockRegistry.of(n.kind).editable) {
      focusBlock(n, 0);
    } else {
      selectBlock(n);
    }
  }

  @override
  Future<void> openEmojiPicker(Node n, BuildContext anchor) async {
    if (readOnly) return;
    final e = await showEmojiPicker(
      context,
      anchor: context.mobile ? null : globalRectOf(anchor),
      removable: false,
    );
    if (e == null || e.isEmpty || !mounted || doc?.byId[n.id] == null) return;
    setTarget(n, e);
  }

  @override
  Future<void> openBlockPicker(Node n, BuildContext anchor) async {
    if (readOnly) return;
    final a = context.mobile ? null : globalRectOf(anchor);
    final Future<String?> pick = switch (n.kind) {
      Kinds.image => pickAttachment(context, a, image: true),
      Kinds.file => pickAttachment(context, a, image: false),
      Kinds.bookmark => pickUrl(context, a),
      Kinds.linkToPage => pickPage(context, a, exclude: widget.pageId),
      embedKind => pickPage(context, a),
      _ => Future.value(null),
    };
    final v = await pick;
    if (v == null || !mounted || doc?.byId[n.id] == null) return;
    setTarget(n, v);
  }

  @override
  void focusCaption(Node n) => focusBlock(n, n.text.length);

  @override
  bool isFocused(Node n) => focusedId == n.id || _pendingFocus == n.id;

  // ---------------------------------------------------------------------
  // Text changes.

  void onChanged(Node n, TextEditingValue v) {
    Bakeoff.instance.textChanged();
    final c = ctl(n);
    final old = n.text;
    final oldMarks = n.marks;
    final text = v.text;
    if (readOnly) return;
    if (!blockRegistry.of(n.kind).multiline &&
        !_softBreak &&
        '\n'.allMatches(text).length > '\n'.allMatches(old).length) {
      // Enter from a soft keyboard arrives as an inserted newline.
      var at = v.selection.baseOffset - 1;
      if (at < 0 || at >= text.length || text[at] != '\n') {
        at = text.indexOf('\n');
      }
      final stripped = text.substring(0, at) + text.substring(at + 1);
      n.marks = shiftMarks(n.marks, old, stripped);
      n.text = stripped;
      c.marks = n.marks;
      c.value = TextEditingValue(
        text: stripped,
        selection: TextSelection.collapsed(offset: at),
      );
      if (stripped != old) _recordTyping(n, old, oldMarks, at);
      if (slashId == n.id) {
        chooseSlash();
      } else if (mentionId == n.id) {
        chooseMention();
      } else {
        split(n, at);
      }
      return;
    }
    _softBreak = false;
    var marks = shiftMarks(n.marks, old, text);
    if (_pending.isNotEmpty &&
        text.length > old.length &&
        v.selection.isCollapsed) {
      final end = v.selection.baseOffset;
      final start = end - (text.length - old.length);
      if (start >= 0) {
        for (final e in _pending.entries) {
          marks = setMark(marks, e.key, start, end, e.value);
        }
      }
    }
    if (text == old && _sameMarks(marks, n.marks)) {
      _updateSlash(n, v);
      _updateMention(n, v);
      return;
    }
    final wasEmptyPage = _emptyPage;
    n.marks = marks;
    n.text = text;
    if (wasEmptyPage != _emptyPage) _safeSetState();
    c.marks = marks;
    _recordTyping(n, old, oldMarks, v.selection.baseOffset);
    _push([n.setTextOp()]);
    if (_markdownShortcut(n, v)) return;
    _updateSlash(n, v);
    _updateMention(n, v, typed: text.length == old.length + 1);
  }

  void onSelectionChanged(
    Node n,
    TextSelection sel,
    SelectionChangedCause? cause,
  ) {
    if (slashId == n.id && !slashTurnInto) {
      final t = n.text;
      final caret = sel.baseOffset;
      if (!sel.isCollapsed ||
          caret < 1 ||
          !t.startsWith('/') ||
          t.substring(1, math.min(caret, t.length)).contains(RegExp(r'\s'))) {
        _safeSetState(() => slashId = null);
      }
    }
    if (mentionId == n.id &&
        (!sel.isCollapsed || sel.baseOffset <= mentionStart)) {
      _safeSetState(() => mentionId = null);
    }
    if (_pending.isNotEmpty) _pending.clear();
    if (!context.mobile) {
      SchedulerBinding.instance.addPostFrameCallback(
        (_) => _updateSelectionToolbar(),
      );
    } else {
      _safeSetState();
    }
  }

  void _updateSelectionToolbar() {
    if (!mounted) return;
    final n = focusedNode;
    Rect? r;
    if (n != null && !_pointerDown && dragNode == null) {
      final sel = ctl(n).selection;
      if (sel.isValid && !sel.isCollapsed) {
        final re = rows[n.id]?.renderEditable;
        if (re != null && re.attached) {
          final boxes = re.getBoxesForSelection(sel);
          if (boxes.isNotEmpty) {
            var u = boxes.first.toRect();
            for (final b in boxes.skip(1)) {
              u = u.expandToInclude(b.toRect());
            }
            r = _toStack(re, u);
          }
        }
      }
    }
    if (r != selRect) setState(() => selRect = r);
  }

  Rect? _toStack(RenderBox box, Rect local) {
    final stack = stackKey.currentContext?.findRenderObject() as RenderBox?;
    if (stack == null) return null;
    final tl = stack.globalToLocal(box.localToGlobal(local.topLeft));
    return tl & local.size;
  }

  // ---------------------------------------------------------------------
  // Structure.

  bool _isList(Node n) => blockRegistry.of(n.kind).listItem;

  void split(Node n, int at) => rec('Split', () => _split(n, at));

  void _split(Node n, int at) {
    final d = doc!;
    final c = ctl(n);
    final text = n.text;
    final type = blockRegistry.of(n.kind);
    if (text.isEmpty && type.exitOnEmptyEnter) {
      // Enter on an empty list item: outdent, or turn into text.
      if (n.depth > 0 && _isList(n)) {
        _push(d.outdent(n));
      } else {
        _push(d.setKind(n, Kinds.paragraph));
      }
      setState(() {});
      focusBlock(n, 0);
      return;
    }
    if (at == 0 && text.isNotEmpty) {
      // Caret at the start: a new empty block above, caret stays.
      final nb = Node(
        id: newBlockId(),
        kind: _isList(n) ? n.kind : Kinds.paragraph,
      );
      final prev = d.prevSibling(n);
      _push(
        prev == null
            ? d.insertFirstChild(n.parent, nb)
            : d.insertAfter(prev, nb),
      );
      setState(() {});
      focusBlock(n, 0);
      return;
    }
    final left = text.substring(0, at), right = text.substring(at);
    final leftMarks = sliceMarks(n.marks, 0, at),
        rightMarks = sliceMarks(n.marks, at, text.length);
    n.text = left;
    n.marks = leftMarks;
    c.marks = leftMarks;
    c.value = TextEditingValue(
      text: left,
      selection: TextSelection.collapsed(offset: left.length),
    );
    final kind = _isList(n) ? n.kind : Kinds.paragraph;
    final nb = Node(
      id: newBlockId(),
      kind: kind,
      text: right,
      marks: rightMarks,
    );
    if (type.children == ChildMode.toggle && isOpen(n)) {
      // Enter in an open toggle: the new block is its first child.
      _push([n.setTextOp(), ...d.insertFirstChild(n, nb)]);
    } else {
      _push([n.setTextOp(), ...d.insertBelow(n, nb)]);
    }
    setState(() {});
    focusBlock(nb, 0);
  }

  void backspaceAtStart(Node n) {
    final d = doc!;
    final type = blockRegistry.of(n.kind);
    if (type.atomic) {
      // An empty caption: select the image (Backspace again deletes it).
      selectBlock(n);
      return;
    }
    if (n.kind != Kinds.paragraph) {
      rec('Turn into text', () => _push(d.setKind(n, Kinds.paragraph)));
      setState(() {});
      return;
    }
    if (n.depth > 0 && d.embedder(n)?.id != n.parent?.parent?.id) {
      rec('Outdent', () => _push(d.outdent(n)));
      setState(() {});
      focusBlock(n, 0);
      return;
    }
    final i = d.reading.indexOf(n);
    if (i > 0) {
      final prev = d.reading[i - 1];
      final pt = blockRegistry.of(prev.kind);
      if (pt.atomic || (!pt.editable && prev.kind != Kinds.divider)) {
        if (n.text.isEmpty && n.children.isEmpty) {
          rec('Delete', () => _push(d.delete(n)));
          _forget(n.id);
        }
        selectBlock(prev);
        return;
      }
    }
    final r = rec('Merge', () {
      final r = d.mergeIntoPrevious(n);
      if (r != null) _push(r.$1);
      return r;
    });
    if (r == null) return;
    final (_, target, caret) = r;
    if (target != n) {
      final tc = ctl(target);
      tc.marks = target.marks;
      tc.value = TextEditingValue(
        text: target.text,
        selection: TextSelection.collapsed(offset: caret),
      );
      _forget(n.id);
    }
    setState(() {});
    focusBlock(target, caret);
  }

  void indent(Node n) {
    final ops = rec('Indent', () {
      final ops = doc!.indent(n);
      _push(ops);
      return ops;
    });
    if (ops == null) return;
    setState(() {});
    focusBlock(n, ctl(n).selection.baseOffset);
  }

  void outdent(Node n) {
    final ops = rec('Outdent', () {
      final ops = doc!.outdent(n);
      _push(ops);
      return ops;
    });
    if (ops == null) return;
    setState(() {});
    focusBlock(n, ctl(n).selection.baseOffset);
  }

  /// The start number and `N. ` prefix typed before [caret] when [text]
  /// begins with one, or null. `1. ` is not one of these: the registry's
  /// numbered prefix already handles it.
  (int, String)? _typedNumberedAt(String text, int caret) {
    final m = RegExp(r'^(\d+)\. ').firstMatch(text);
    if (m == null || m.end != caret) return null;
    final start = int.tryParse(m.group(1)!);
    if (start == null || start > maxNumberedStart) return null;
    return (start, m.group(0)!);
  }

  bool _markdownShortcut(Node n, TextEditingValue v) {
    if (n.kind != Kinds.paragraph || !v.selection.isCollapsed) return false;
    final hit = blockRegistry.markdownAt(n.text, v.selection.baseOffset);
    final typed = hit == null ? _typedNumberedAt(n.text, v.selection.baseOffset) : null;
    if (hit == null && typed == null) return false;
    final type = hit?.$1 ?? blockRegistry.of(Kinds.numbered);
    final prefix = hit?.$2 ?? typed!.$2;
    final d = doc!;
    final c = ctl(n);
    rec('Turn into', () {
      final rest = n.text.substring(prefix.length);
      n.marks = shiftMarks(n.marks, n.text, rest);
      n.text = rest;
      c.marks = n.marks;
      c.value = TextEditingValue(
        text: rest,
        selection: const TextSelection.collapsed(offset: 0),
      );
      if (!type.editable) {
        _push([n.setTextOp(), ..._turnInto(n, type.kind)]);
      } else {
        final ops = [n.setTextOp(), ...d.setKind(n, type.kind)];
        // A typed start joins the run above when there is one, and otherwise
        // starts a new run at that number.
        final start = typed?.$1;
        if (start != null && start != 1 && d.prevSibling(n)?.kind != Kinds.numbered) {
          ops.addAll(d.setNumberedStart(n, start));
        }
        _push(ops);
      }
    });
    _typingEntry = null;
    slashId = null;
    setState(() {});
    return true;
  }

  /// Turns [n] into [kind]. A block without text (a divider) goes below a
  /// non-empty [n], and the caret moves to a paragraph after it.
  List<BOp> _turnInto(Node n, String kind) {
    final d = doc!;
    final type = blockRegistry.of(kind);
    final storedKind = type.insertKind ?? kind;
    if (type.editable && !type.atomic) {
      final ops = d.setKind(n, kind);
      if (type.children == ChildMode.toggle) _open.add(n.id);
      focusBlock(n, ctl(n).selection.baseOffset);
      return ops;
    }
    final ops = <BOp>[];
    Node block;
    if (n.text.isEmpty && n.children.isEmpty) {
      ops.addAll(d.setKind(n, storedKind));
      block = n;
      fn(n).unfocus();
    } else {
      block = Node(id: newBlockId(), kind: storedKind);
      ops.addAll(d.insertAfter(n, block));
    }
    final next = d.nextSibling(block);
    if (next != null && next.kind == Kinds.paragraph && next.text.isEmpty) {
      focusBlock(next, 0);
    } else {
      final p = Node(id: newBlockId(), kind: Kinds.paragraph);
      ops.addAll(d.insertAfter(block, p));
      focusBlock(p, 0);
    }
    final createTarget = type.createTarget;
    if (createTarget != null) {
      createTarget(context, pageId).then((target) {
        if (target != null && mounted && doc?.byId[block.id] == block) {
          setTarget(block, target);
        }
      });
    } else if (type.atomic ||
        kind == Kinds.linkToPage ||
        (kind == embedKind &&
            (block.target == null || block.target!.isEmpty))) {
      _openPickerSoon(block);
    }
    return ops;
  }

  /// Opens the picker of a media block once its row is built.
  void _openPickerSoon(Node n) {
    SchedulerBinding.instance.addPostFrameCallback((_) {
      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        final r = rows[n.id];
        openBlockPicker(n, r != null && r.mounted ? r.context : context);
      });
    });
  }

  void blockTurnInto(Node n, String kind) {
    rec('Turn into', () => _push(_turnInto(n, kind)));
    setState(() {});
  }

  @override
  void toggleChecked(Node n) {
    if (readOnly) return;
    final completing = !n.checked;
    final before = <String, HeadingProgress?>{
      if (completing)
        for (final heading in doc!.flat)
          if (headingLevel(heading.kind) != null)
            heading.id: headingProgressOf(heading, doc!.flat),
    };
    rec('Check', () => _push(doc!.setChecked(n, !n.checked)));
    if (completing) {
      for (final heading in doc!.flat) {
        final old = before[heading.id];
        if (old == null || old.done == old.total) continue;
        final next = headingProgressOf(heading, doc!.flat);
        if (next != null && next.done == next.total) {
          _headingCompletions.update(
            heading.id,
            (n) => n + 1,
            ifAbsent: () => 1,
          );
        }
      }
    }
    setState(() {});
  }

  void addBlockBelow(Node n) {
    final nb = Node(id: newBlockId(), kind: Kinds.paragraph);
    rec('Add block', () => _push(doc!.insertBelow(n, nb)));
    setState(() {
      menuBlockId = null;
      slashId = nb.id;
      slashQuery = '';
      slashIndex = 0;
      slashTurnInto = true;
      slashAnchor = null;
    });
    focusBlock(nb, 0);
    SchedulerBinding.instance.addPostFrameCallback((_) {
      SchedulerBinding.instance.addPostFrameCallback(
        (_) => _anchorSlash(nb, 0),
      );
    });
  }

  /// Inserts a column list with [count] columns after [n] (in place of an
  /// empty paragraph) and puts the caret in the first column.
  void insertColumns(Node n, int count) {
    final d = doc!;
    final list = Node(id: newBlockId(), kind: Kinds.columnList);
    Node? first;
    for (var i = 0; i < count; i++) {
      final col = Node(id: newBlockId(), kind: Kinds.column)..parent = list;
      final p = Node(id: newBlockId(), kind: Kinds.paragraph)..parent = col;
      col.children.add(p);
      list.children.add(col);
      first ??= p;
    }
    rec('Columns', () {
      _push(d.insertAfter(n, list));
      if (n.kind == Kinds.paragraph && n.text.isEmpty && n.children.isEmpty) {
        _push(d.delete(n));
        _forget(n.id);
      }
    });
    setState(() {});
    focusBlock(first!, 0);
  }

  /// Creates a sub-page, links it from [n] and opens it.
  Future<void> insertSubPage(Node n) async {
    final vault = context.app.vault;
    final id = await vault.createPage(parent: widget.pageId);
    if (!mounted || doc?.byId[n.id] == null) return;
    final d = doc!;
    rec('Add page', () {
      if (n.text.isEmpty && n.children.isEmpty && n.kind == Kinds.paragraph) {
        _push(d.setKind(n, Kinds.linkToPage));
        _push(d.setTarget(n, id));
      } else {
        final b = Node(id: newBlockId(), kind: Kinds.linkToPage, target: id);
        _push(d.insertAfter(n, b));
      }
    });
    setState(() {});
    _saveTitle();
    await session.save();
    if (!mounted) return;
    navOpenPage(context, id, '', isNew: true, stack: true);
  }

  void blockAction(Node n, BlockAction a, [String? kind]) {
    final d = doc!;
    final targets = selected.contains(n.id) && selected.length > 1
        ? _selectedTops()
        : [n];
    switch (a) {
      case BlockAction.moveUp:
        _moveBlocks(targets, -1);
      case BlockAction.moveDown:
        _moveBlocks(targets, 1);
      case BlockAction.duplicate:
        _duplicate(targets);
      case BlockAction.delete:
        _deleteBlocks(targets);
      case BlockAction.copy:
        copyBlocks(targets);
      case BlockAction.turnInto:
        if (kind != null) {
          if (targets.length == 1) {
            blockTurnInto(n, kind);
          } else {
            rec('Turn into', () {
              for (final t in _selectedAll()) {
                if (blockRegistry.of(t.kind).editable &&
                    !blockRegistry.of(t.kind).atomic) {
                  _push(d.setKind(t, kind));
                }
              }
            });
          }
        }
      case BlockAction.color:
        if (kind != null) {
          _colorBlocks(
            selected.contains(n.id) && selected.length > 1
                ? _selectedAll()
                : [n],
            kind,
          );
        }
      case BlockAction.setDate:
        final anchor = rows[n.id]?.context;
        if (anchor != null) {
          editTodoDate(context, globalRectOf(anchor), n, (edit) {
            if (!mounted || readOnly || doc?.byId[n.id] != n) return;
            rec('Set date', () {
              n.text = edit.text;
              n.marks = edit.marks;
              final c = ctl(n);
              c.marks = n.marks;
              c.value = TextEditingValue(
                text: n.text,
                selection: TextSelection.collapsed(offset: n.text.length),
              );
              _push([n.setTextOp()]);
            });
            setState(() {});
          });
        }
      case BlockAction.select:
        setState(() {
          selectMode = true;
          selected
            ..clear()
            ..add(n.id);
        });
        return;
    }
    setState(() {
      menuBlockId = null;
      if (a == BlockAction.delete || a == BlockAction.copy) selected.clear();
    });
  }

  @override
  void selectBlock(Node n) {
    FocusManager.instance.primaryFocus?.unfocus();
    setState(() {
      selected
        ..clear()
        ..add(n.id);
      _selAnchor = n.id;
    });
    _pageFocus.requestFocus();
  }

  /// A tap on a block without text: select it (shift extends).
  void tapBlock(Node n) {
    if (selectMode) {
      toggleSelected(n);
      return;
    }
    if (HardwareKeyboard.instance.isShiftPressed && extendSelectionTo(n)) {
      return;
    }
    selectBlock(n);
  }

  bool isSelected(Node n) => selected.contains(n.id);

  void toggleSelected(Node n) {
    setState(() {
      if (!selected.remove(n.id)) selected.add(n.id);
    });
  }

  void clearBlockSelection() {
    if (selected.isNotEmpty) _safeSetState(() => selected.clear());
  }

  /// Shift-click: selects the blocks from the focused or anchor block to
  /// [n]. Returns false when there is nothing to extend from.
  bool extendSelectionTo(Node n) {
    final d = doc;
    final from = d?.byId[focusedId ?? _selAnchor ?? ''];
    if (d == null || from == null) return false;
    final a = from.flatIndex, b = n.flatIndex;
    FocusManager.instance.primaryFocus?.unfocus();
    setState(() {
      selected
        ..clear()
        ..addAll([
          for (var i = math.min(a, b); i <= math.max(a, b); i++) d.flat[i].id,
        ]);
      _selAnchor = from.id;
    });
    _pageFocus.requestFocus();
    return true;
  }

  /// The selected blocks without those inside other selected blocks, in
  /// page order.
  List<Node> _selectedTops() {
    final d = doc;
    if (d == null) return const [];
    return [
      for (final n in d.flat)
        if (selected.contains(n.id) && !_hasSelectedAncestor(n)) n,
    ];
  }

  List<Node> _selectedAll() => [
    for (final n in doc?.flat ?? const <Node>[])
      if (selected.contains(n.id)) n,
  ];

  bool _hasSelectedAncestor(Node n) {
    for (var p = n.parent; p != null; p = p.parent) {
      if (selected.contains(p.id)) return true;
    }
    return false;
  }

  void _deleteBlocks(List<Node> tops) {
    if (tops.isEmpty || readOnly) return;
    final d = doc!;
    rec('Delete', () {
      for (final n in tops) {
        void forget(Node x) {
          _forget(x.id);
          x.children.forEach(forget);
        }

        forget(n);
        _push(d.delete(n));
      }
      if (d.flat.isEmpty) {
        _push(
          d.insertFirstChild(
            null,
            Node(id: newBlockId(), kind: Kinds.paragraph),
          ),
        );
      }
    });
    setState(() => selected.clear());
  }

  void _duplicate(List<Node> tops) {
    if (tops.isEmpty || readOnly) return;
    final d = doc!;
    final copies = <String>[];
    rec('Duplicate', () {
      for (final n in tops.reversed) {
        final c = n.copy();
        copies.add(c.id);
        _push(d.insertAfter(n, c));
      }
    });
    setState(() {
      selected
        ..clear()
        ..addAll(copies);
    });
  }

  void _moveBlocks(List<Node> tops, int dir) {
    if (tops.isEmpty || readOnly) return;
    final d = doc!;
    rec('Move', () {
      final list = dir < 0 ? tops : tops.reversed.toList();
      for (final n in list) {
        final ops = dir < 0 ? d.moveUp(n) : d.moveDown(n);
        if (ops == null) break;
        _push(ops);
      }
    });
    setState(() {});
  }

  void _colorBlocks(List<Node> nodes, String color) {
    if (readOnly) return;
    final d = doc!;
    final v = color == 'default' || color == 'default_background'
        ? null
        : color;
    rec('Color', () {
      for (final n in nodes) {
        if (blockRegistry.of(n.kind).colorable) _push(d.setColor(n, v));
      }
    });
    setState(() {});
  }

  void _indentSelected(bool out) {
    final tops = _selectedTops();
    if (tops.isEmpty || readOnly) return;
    final d = doc!;
    rec(out ? 'Outdent' : 'Indent', () {
      for (final n in out ? tops.reversed : tops) {
        _push(out ? d.outdent(n) : d.indent(n));
      }
    });
    setState(() {});
  }

  void selectAll() {
    final d = doc;
    if (d == null) return;
    FocusManager.instance.primaryFocus?.unfocus();
    setState(() {
      selected
        ..clear()
        ..addAll(d.flat.map((n) => n.id));
    });
    _pageFocus.requestFocus();
  }

  /// Pastes markdown blocks after [after] (replacing it when it is an
  /// empty paragraph), or after the last block.
  void pasteBlocks(List<Node> nodes, {Node? after}) {
    if (nodes.isEmpty || readOnly) return;
    final d = doc!;
    var anchor = after ?? (d.roots.isEmpty ? null : d.roots.last);
    Node? last;
    rec('Paste', () {
      for (final n in nodes) {
        if (anchor == null) {
          _push(d.insertFirstChild(null, n));
        } else {
          _push(d.insertAfter(anchor!, n));
        }
        anchor = n;
        last = n;
      }
      if (after != null &&
          after.kind == Kinds.paragraph &&
          after.text.isEmpty &&
          after.children.isEmpty) {
        _push(d.delete(after));
        _forget(after.id);
      }
    });
    setState(() => selected.clear());
    final l = last;
    if (l != null) {
      final end = d.lastDescendant(l);
      if (blockRegistry.of(end.kind).editable) focusBlock(end, end.text.length);
    }
  }

  Future<void> _pasteInto(Node n) async {
    final text = await readClipboardText();
    if (text == null || text.isEmpty || !mounted || doc?.byId[n.id] == null) {
      return;
    }
    final c = ctl(n);
    final multi = text.contains('\n') && !blockRegistry.of(n.kind).multiline;
    if (!multi) {
      final sel = c.selection.isValid
          ? c.selection
          : TextSelection.collapsed(offset: c.text.length);
      final v = c.value.replaced(sel, text);
      c.value = v;
      onChanged(n, v);
      return;
    }
    final nodes = markdownToNodes(text);
    if (nodes.isEmpty) return;
    if (n.text.isNotEmpty &&
        nodes.length == 1 &&
        nodes.first.kind == Kinds.paragraph) {
      final sel = c.selection.isValid
          ? c.selection
          : TextSelection.collapsed(offset: c.text.length);
      final v = c.value.replaced(sel, nodes.first.text);
      c.value = v;
      onChanged(n, v);
      return;
    }
    pasteBlocks(nodes, after: n);
  }

  @override
  Future<void> onLongPress(Node n) async {
    if (readOnly) return;
    HapticFeedback.lightImpact();
    FocusManager.instance.primaryFocus?.unfocus();
    final multi = selectMode && selected.length > 1 && selected.contains(n.id);
    if (!multi) {
      setState(() {
        selected
          ..clear()
          ..add(n.id);
      });
    }
    var askedAgent = false;
    final r = await showSheet<SheetResult>(
      context,
      (_) => BlockSheet(
        kind: n.kind,
        count: multi ? selected.length : 1,
        color: n.color,
        onAskAi: readOnly
            ? null
            : () {
                askedAgent = true;
                askAiForSelection(block: n);
              },
        onAskAgent: _canAskAgent
            ? () {
                askedAgent = true;
                askAgentForSelection(block: n);
              }
            : null,
      ),
    );
    if (!mounted) return;
    if (askedAgent) return;
    if (r != null && doc!.byId.containsKey(n.id)) {
      blockAction(n, r.action, r.kind);
    }
    if (!selectMode) setState(() => selected.clear());
  }

  void openBlockMenu(Node n, Rect handlesGlobal) {
    final stack = stackKey.currentContext?.findRenderObject() as RenderBox?;
    if (stack == null) return;
    final tl = stack.globalToLocal(handlesGlobal.topLeft);
    FocusManager.instance.primaryFocus?.unfocus();
    setState(() {
      menuBlockId = n.id;
      if (!selected.contains(n.id)) {
        selected
          ..clear()
          ..add(n.id);
      }
      menuAnchor = tl & handlesGlobal.size;
    });
  }

  void closeBlockMenu() => setState(() {
    menuBlockId = null;
    selected.clear();
  });

  // ---------------------------------------------------------------------
  // `/` menu.

  List<BlockType> get _slashTypes => slashTurnInto
      ? blockRegistry.turnIntoTypes
      : [
          ...blockRegistry.filter(slashQuery),
          for (final item in appAddonContext.slashItems.all)
            if (item.blockKind == null &&
                item.label.toLowerCase().contains(
                  slashQuery.trim().toLowerCase(),
                ))
              BlockType(
                kind: item.id,
                label: item.label,
                hint: item.description,
                group: slashQuery.trim().isEmpty
                    ? item.group
                    : BlockRegistry.resultsGroup,
                insertOnly: true,
                codecs: genericBlockCodecs,
              ),
        ];

  void _updateSlash(Node n, TextEditingValue v) {
    if (slashTurnInto && slashId == n.id) {
      if (n.text.isEmpty) return;
      slashTurnInto =
          false; // typing into the "+" block filters like a normal `/`
      if (!n.text.startsWith('/')) {
        setState(() => slashId = null);
        return;
      }
    }
    final t = n.text;
    final caret = v.selection.baseOffset;
    if (t.startsWith('/') &&
        caret >= 1 &&
        caret <= t.length &&
        !t.substring(1, caret).contains(RegExp(r'\s'))) {
      final q = t.substring(1, caret);
      if (blockRegistry.filter(q).isEmpty && q.length > 3) {
        setState(() => slashId = null);
        return;
      }
      final opening = slashId != n.id;
      setState(() {
        slashId = n.id;
        slashQuery = q;
        slashTurnInto = false;
        if (opening) slashIndex = 0;
        slashIndex = slashIndex.clamp(
          0,
          math.max(0, blockRegistry.filter(q).length - 1),
        );
      });
      if (opening) {
        rows[n.id]?.editableTextKey.currentState?.hideToolbar();
        SchedulerBinding.instance.addPostFrameCallback(
          (_) => _anchorSlash(n, 1),
        );
      }
    } else if (slashId == n.id) {
      setState(() => slashId = null);
    }
  }

  Rect? _caretRect(Node n, int offset) {
    final re = rows[n.id]?.renderEditable;
    if (re == null || !re.attached || !mounted) return null;
    final caret = re.getLocalRectForCaret(
      TextPosition(offset: math.min(offset, n.text.length)),
    );
    return _toStack(re, caret);
  }

  void _anchorSlash(Node n, int offset) {
    final a = _caretRect(n, offset);
    if (a == null) return;
    final stack = stackKey.currentContext?.findRenderObject() as RenderBox?;
    if (context.mobile && stack != null && scroll.hasClients) {
      // Keep room for four rows of the menu below the caret line.
      final need = a.bottom + 8 + 4 * 44 + 46 + 8 - stack.size.height;
      if (need > 0 &&
          scroll.offset + need <= scroll.position.maxScrollExtent + 0.5) {
        scroll.jumpTo(scroll.offset + need);
        setState(() => slashAnchor = a.shift(Offset(0, -need)));
        return;
      }
    }
    setState(() => slashAnchor = a);
  }

  void openTurnIntoMenu() {
    final n = focusedNode;
    if (n == null) return;
    setState(() {
      slashId = n.id;
      slashQuery = '';
      slashIndex = math.max(
        0,
        blockRegistry.turnIntoTypes.indexWhere((t) => t.kind == n.kind),
      );
      slashTurnInto = true;
      selRect = null;
    });
    _anchorSlash(n, 0);
  }

  void closeSlash() => setState(() => slashId = null);

  void chooseSlash([String? chosen]) {
    final d = doc;
    final id = slashId;
    if (d == null || id == null) return;
    final n = d.byId[id];
    final types = _slashTypes;
    final kind =
        chosen ??
        (types.isEmpty
            ? null
            : types[slashIndex.clamp(0, types.length - 1)].kind);
    final wasTurnInto = slashTurnInto;
    setState(() => slashId = null);
    if (n == null || kind == null) return;
    rec('Turn into', () {
      if (!wasTurnInto && n.text.startsWith('/')) {
        final c = ctl(n);
        final end = math.min(1 + slashQuery.length, n.text.length);
        final rest = n.text.substring(end);
        n.marks = shiftMarks(n.marks, n.text, rest);
        n.text = rest;
        c.marks = n.marks;
        c.value = TextEditingValue(
          text: rest,
          selection: const TextSelection.collapsed(offset: 0),
        );
        _push([n.setTextOp()]);
      }
      switch (kind) {
        case 'columns2':
          insertColumns(n, 2);
        case 'columns3':
          insertColumns(n, 3);
        case 'subpage':
          insertSubPage(n);
        case 'askai':
          break;
        default:
          if (appAddonContext.slashItems.of(kind) == null ||
              appAddonContext.slashItems.of(kind)?.blockKind != null) {
            _push(_turnInto(n, kind));
          }
      }
    });
    _typingEntry = null;
    setState(() {});
    if (kind == 'askai') showAskAi(context, pageId: pageId, after: n.id);
    appAddonContext.slashItems.of(kind)?.run(context);
  }

  // ---------------------------------------------------------------------
  // `@` menu, and `[[` for its pages alone.

  void _updateMention(Node n, TextEditingValue v, {bool typed = false}) {
    final t = n.text;
    final caret = v.selection.baseOffset;
    if (mentionId != n.id) {
      if (!typed || !v.selection.isCollapsed || caret < 1 || caret > t.length) {
        return;
      }
      final brackets = caret >= 2 && t.substring(caret - 2, caret) == '[[';
      if (!brackets) {
        if (t[caret - 1] != '@') return;
        if (caret >= 2 && !RegExp(r'\s').hasMatch(t[caret - 2])) return;
      }
      setState(() {
        mentionId = n.id;
        mentionPagesOnly = brackets;
        mentionStart = caret - (brackets ? 2 : 1);
        mentionQuery = '';
        mentionIndex = 0;
        mentionItems = brackets ? const [] : dateItems('', clockNow());
        mentionAnchor = null;
      });
      _searchMentions();
      SchedulerBinding.instance.addPostFrameCallback((_) {
        final a = _caretRect(n, caret);
        if (a != null && mounted) setState(() => mentionAnchor = a);
      });
      return;
    }
    final trigger = _mentionTrigger;
    if (caret < mentionStart + trigger.length ||
        mentionStart >= t.length ||
        !t.startsWith(trigger, mentionStart) ||
        caret > t.length) {
      setState(() => mentionId = null);
      return;
    }
    final q = t.substring(mentionStart + trigger.length, caret);
    if (q.contains('\n') || q.length > 40 || q.startsWith(' ')) {
      setState(() => mentionId = null);
      return;
    }
    setState(() => mentionQuery = q);
    _searchMentions();
  }

  Future<void> _searchMentions() async {
    final g = ++_mentionGen;
    final q = mentionQuery;
    final pages = await mentionPages(context, q, widget.pageId);
    if (!mounted || g != _mentionGen || mentionId == null) return;
    setState(() {
      mentionItems = [
        if (!mentionPagesOnly) ...dateItems(q, clockNow()),
        ...pages,
      ];
      mentionIndex = mentionIndex.clamp(
        0,
        math.max(0, mentionItems.length - 1),
      );
    });
  }

  /// Opens the `@` menu at the caret of the focused block (toolbar button).
  void startMention() {
    final n = focusedNode;
    if (n == null) return;
    final c = ctl(n);
    final sel = c.selection.isValid
        ? c.selection
        : TextSelection.collapsed(offset: c.text.length);
    final v = c.value.replaced(sel, '@');
    c.value = v;
    onChanged(n, v);
  }

  void chooseMention([MentionItem? chosen]) {
    final d = doc;
    final id = mentionId;
    if (d == null || id == null) return;
    final n = d.byId[id];
    final item =
        chosen ??
        (mentionItems.isEmpty
            ? null
            : mentionItems[mentionIndex.clamp(0, mentionItems.length - 1)]);
    setState(() => mentionId = null);
    if (n == null || item == null) return;
    final c = ctl(n);
    final start = mentionStart;
    final end = math.min(
      start + _mentionTrigger.length + mentionQuery.length,
      n.text.length,
    );
    final insert = item.isDate
        ? '@${longDate(item.date!)}'
        : item.isChat
        ? displayTitle(item.label)
        : pageLabel(
            _sheetNavigator,
            item.id,
            item.label,
            firstLine: _meta.meta(item.id)?.firstLine ?? '',
          ).text;
    rec('Mention', () {
      final text = n.text.substring(0, start) + insert + n.text.substring(end);
      var marks = shiftMarks(
        n.marks,
        n.text,
        n.text.substring(0, start) + n.text.substring(end),
      );
      marks = shiftMarks(
        marks,
        n.text.substring(0, start) + n.text.substring(end),
        text,
      );
      marks = setMark(
        marks,
        item.isDate ? BMarkKind.date : BMarkKind.pageLink,
        start,
        start + insert.length,
        true,
        url: item.isDate ? isoDate(item.date!) : item.id,
      );
      n.text = text;
      n.marks = marks;
      c.marks = marks;
      c.value = TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: start + insert.length),
      );
      _push([n.setTextOp()]);
    });
    focusBlock(n, start + insert.length);
  }

  /// A tap at [offset] in [n]'s text: opens a mentioned page. Returns true
  /// when it did.
  bool openMentionAt(Node n, int offset) {
    for (final m in n.marks) {
      final target = inlinePageTarget(m);
      if (target != null && m.start <= offset && offset < m.end) {
        if (focusedId == n.id &&
            !readOnly &&
            isCommandKeyPressed) {
          return false;
        }
        openPage(target);
        return true;
      }
    }
    return false;
  }

  // ---------------------------------------------------------------------
  // Keys.

  bool _ctrl() =>
      HardwareKeyboard.instance.isControlPressed ||
      HardwareKeyboard.instance.isMetaPressed;

  /// Undo, redo (in every block, the title and the page).
  bool _historyKey(LogicalKeyboardKey k, bool ctrl, bool shift) {
    if (!ctrl) return false;
    if (k == LogicalKeyboardKey.keyZ) {
      shift ? redo() : undo();
      return true;
    }
    if (k == LogicalKeyboardKey.keyY && !shift) {
      redo();
      return true;
    }
    return false;
  }

  KeyEventResult onKey(Node n, KeyEvent e) {
    if (e is! KeyDownEvent && e is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final k = e.logicalKey;
    final hk = HardwareKeyboard.instance;
    final ctrl = _ctrl();
    final shift = hk.isShiftPressed;
    final c = ctl(n);
    final sel = c.selection;
    final isEnter =
        k == LogicalKeyboardKey.enter || k == LogicalKeyboardKey.numpadEnter;

    if (_historyKey(k, ctrl, shift)) return KeyEventResult.handled;
    if (readOnly) {
      if (k == LogicalKeyboardKey.escape) {
        selectBlock(n);
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    }

    if (slashId == n.id) {
      final count = _slashTypes.length;
      if (k == LogicalKeyboardKey.arrowDown && count > 0) {
        setState(() => slashIndex = (slashIndex + 1) % count);
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.arrowUp && count > 0) {
        setState(() => slashIndex = (slashIndex - 1 + count) % count);
        return KeyEventResult.handled;
      }
      if (isEnter) {
        chooseSlash();
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.escape) {
        closeSlash();
        return KeyEventResult.handled;
      }
    }

    if (mentionId == n.id) {
      final count = mentionItems.length;
      if (k == LogicalKeyboardKey.arrowDown && count > 0) {
        setState(() => mentionIndex = (mentionIndex + 1) % count);
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.arrowUp && count > 0) {
        setState(() => mentionIndex = (mentionIndex - 1 + count) % count);
        return KeyEventResult.handled;
      }
      if (isEnter || k == LogicalKeyboardKey.tab) {
        chooseMention();
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.escape) {
        setState(() => mentionId = null);
        return KeyEventResult.handled;
      }
    }

    if (isEnter) {
      if (c.value.composing.isValid && !c.value.composing.isCollapsed) {
        return KeyEventResult.ignored;
      }
      if (shift || blockRegistry.of(n.kind).multiline) {
        _softBreak = true;
        final v = c.value.replaced(sel, '\n');
        c.value = v;
        onChanged(n, v);
        return KeyEventResult.handled;
      }
      var at = sel.start;
      if (!sel.isCollapsed) {
        final v = c.value.replaced(sel, '');
        c.value = v;
        onChanged(n, v);
        at = v.selection.baseOffset;
      }
      if (blockRegistry.of(n.kind).atomic) {
        // Enter in a caption: a new paragraph below the media block.
        final p = Node(id: newBlockId(), kind: Kinds.paragraph);
        rec('Add block', () => _push(doc!.insertAfter(n, p)));
        setState(() {});
        focusBlock(p, 0);
        return KeyEventResult.handled;
      }
      split(n, at);
      return KeyEventResult.handled;
    }

    if (k == LogicalKeyboardKey.backspace &&
        sel.isCollapsed &&
        sel.baseOffset == 0 &&
        !ctrl) {
      if (c.value.composing.isValid && !c.value.composing.isCollapsed) {
        return KeyEventResult.ignored;
      }
      backspaceAtStart(n);
      return KeyEventResult.handled;
    }

    if (k == LogicalKeyboardKey.tab) {
      shift ? outdent(n) : indent(n);
      return KeyEventResult.handled;
    }

    if (ctrl &&
        shift &&
        (k == LogicalKeyboardKey.arrowUp ||
            k == LogicalKeyboardKey.arrowDown)) {
      _moveBlocks([n], k == LogicalKeyboardKey.arrowUp ? -1 : 1);
      focusBlock(n, sel.baseOffset);
      return KeyEventResult.handled;
    }

    if (ctrl && !shift) {
      final mk = switch (k) {
        LogicalKeyboardKey.keyB => BMarkKind.bold,
        LogicalKeyboardKey.keyI => BMarkKind.italic,
        LogicalKeyboardKey.keyU => BMarkKind.underline,
        LogicalKeyboardKey.keyE => BMarkKind.code,
        _ => null,
      };
      if (mk != null) {
        toggleMark(mk);
        return KeyEventResult.handled;
      }
      // With a selection Ctrl+K adds a link; otherwise it opens search.
      if (k == LogicalKeyboardKey.keyK && !sel.isCollapsed) {
        openLink();
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.keyA &&
          (n.text.isEmpty || (sel.start == 0 && sel.end == n.text.length))) {
        selectAll();
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.keyV) {
        _pasteInto(n);
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.keyD) {
        _duplicate([n]);
        return KeyEventResult.handled;
      }
    }
    if (ctrl && shift && k == LogicalKeyboardKey.keyS) {
      toggleMark(BMarkKind.strike);
      return KeyEventResult.handled;
    }

    final re = rows[n.id]?.renderEditable;
    if (shift &&
        !ctrl &&
        re != null &&
        (k == LogicalKeyboardKey.arrowUp ||
            k == LogicalKeyboardKey.arrowDown)) {
      // Shift+arrow past the first or last line selects whole blocks.
      final r = re.getLocalRectForCaret(sel.extent);
      final atEdge = k == LogicalKeyboardKey.arrowUp
          ? r.top < re.preferredLineHeight / 2
          : r.bottom > re.size.height - re.preferredLineHeight / 2;
      if (atEdge) {
        selectBlock(n);
        _extendSelection(k == LogicalKeyboardKey.arrowUp ? -1 : 1);
        return KeyEventResult.handled;
      }
    }

    if (!ctrl && !shift && sel.isCollapsed) {
      if (k == LogicalKeyboardKey.arrowUp && re != null) {
        final r = re.getLocalRectForCaret(sel.extent);
        if (r.top < re.preferredLineHeight / 2) {
          final prev = _adjacentEditable(n, -1);
          if (prev != null) {
            focusBlock(prev, prev.text.length);
            return KeyEventResult.handled;
          }
        }
      }
      if (k == LogicalKeyboardKey.arrowDown && re != null) {
        final r = re.getLocalRectForCaret(sel.extent);
        if (r.bottom > re.size.height - re.preferredLineHeight / 2) {
          final next = _adjacentEditable(n, 1);
          if (next != null) {
            focusBlock(next, next.text.length);
            return KeyEventResult.handled;
          }
        }
      }
      if (k == LogicalKeyboardKey.arrowLeft && sel.baseOffset == 0) {
        final prev = _adjacentEditable(n, -1);
        if (prev != null) {
          focusBlock(prev, prev.text.length);
          return KeyEventResult.handled;
        }
      }
      if (k == LogicalKeyboardKey.arrowRight &&
          sel.baseOffset == n.text.length) {
        final next = _adjacentEditable(n, 1);
        if (next != null) {
          focusBlock(next, 0);
          return KeyEventResult.handled;
        }
      }
    }

    if (k == LogicalKeyboardKey.escape) {
      if (linkTarget != null) {
        setState(() => linkTarget = null);
        return KeyEventResult.handled;
      }
      selectBlock(n);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// Keys while blocks are selected (the page has focus).
  KeyEventResult _onPageKey(KeyEvent e) {
    if (e is! KeyDownEvent && e is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final k = e.logicalKey;
    final ctrl = _ctrl();
    final shift = HardwareKeyboard.instance.isShiftPressed;
    if (_historyKey(k, ctrl, shift)) return KeyEventResult.handled;
    if (selected.isEmpty) return KeyEventResult.ignored;
    final tops = _selectedTops();
    if (k == LogicalKeyboardKey.escape) {
      setState(() {
        selected.clear();
        selectMode = false;
      });
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.backspace || k == LogicalKeyboardKey.delete) {
      _deleteBlocks(tops);
      return KeyEventResult.handled;
    }
    if (ctrl && k == LogicalKeyboardKey.keyC) {
      copyBlocks(tops);
      return KeyEventResult.handled;
    }
    if (ctrl && k == LogicalKeyboardKey.keyX) {
      copyBlocks(tops);
      _deleteBlocks(tops);
      return KeyEventResult.handled;
    }
    if (ctrl && k == LogicalKeyboardKey.keyD) {
      _duplicate(tops);
      return KeyEventResult.handled;
    }
    if (ctrl && k == LogicalKeyboardKey.keyA) {
      selectAll();
      return KeyEventResult.handled;
    }
    if (ctrl && k == LogicalKeyboardKey.keyV) {
      readClipboardText().then((t) {
        if (t == null || !mounted) return;
        pasteBlocks(markdownToNodes(t), after: tops.isEmpty ? null : tops.last);
      });
      return KeyEventResult.handled;
    }
    if (ctrl &&
        shift &&
        (k == LogicalKeyboardKey.arrowUp ||
            k == LogicalKeyboardKey.arrowDown)) {
      _moveBlocks(tops, k == LogicalKeyboardKey.arrowUp ? -1 : 1);
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.arrowUp || k == LogicalKeyboardKey.arrowDown) {
      final dir = k == LogicalKeyboardKey.arrowUp ? -1 : 1;
      if (shift) {
        _extendSelection(dir);
      } else {
        final d = doc!;
        final list = d.visible;
        final cur = tops.isEmpty ? null : (dir < 0 ? tops.first : tops.last);
        final i = cur == null ? -1 : list.indexOf(d.embedder(cur) ?? cur);
        final j = (i + dir).clamp(0, list.length - 1);
        if (list.isNotEmpty) selectBlock(list[j]);
      }
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.tab) {
      _indentSelected(shift);
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.enter &&
        tops.length == 1 &&
        blockRegistry.of(tops.first.kind).editable) {
      final n = tops.first;
      setState(() => selected.clear());
      focusBlock(n, n.text.length);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// Grows or shrinks the block selection by one block in [dir] (from the
  /// anchor, as text selection does).
  void _extendSelection(int dir) {
    final d = doc;
    if (d == null) return;
    final list = d.visible;
    final anchor = d.byId[_selAnchor ?? ''];
    if (anchor == null) return;
    final a = list.indexOf(d.embedder(anchor) ?? anchor);
    final idx = [
      for (var i = 0; i < list.length; i++)
        if (selected.contains(list[i].id)) i,
    ];
    if (a < 0 || idx.isEmpty) return;
    final lo = idx.first, hi = idx.last;
    // The far end moves.
    var far = (hi > a ? hi : lo) + dir;
    if (lo < a && hi == a) far = lo + dir;
    far = far.clamp(0, list.length - 1);
    final from = math.min(a, far), to = math.max(a, far);
    setState(() {
      selected.clear();
      for (var i = from; i <= to; i++) {
        selected.add(list[i].id);
        for (final x in d.rowsUnder(list[i])) {
          selected.add(x.id);
        }
      }
    });
  }

  Node? _adjacentEditable(Node n, int dir) {
    final f = doc!.reading;
    final i = f.indexOf(n);
    for (var j = i + dir; j >= 0 && j < f.length; j += dir) {
      if (blockRegistry.of(f[j].kind).editable) return f[j];
    }
    return null;
  }

  KeyEventResult _onTitleKey(FocusNode node, KeyEvent e) {
    if (e is! KeyDownEvent) return KeyEventResult.ignored;
    if (_ctrl() && e.logicalKey == LogicalKeyboardKey.keyZ) {
      return KeyEventResult.ignored;
    }
    if (e.logicalKey == LogicalKeyboardKey.enter ||
        e.logicalKey == LogicalKeyboardKey.arrowDown) {
      _focusFirstBlock();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  void _focusFirstBlock() {
    final d = doc;
    if (d == null) return;
    if (d.flat.isEmpty || !blockRegistry.of(d.flat.first.kind).editable) {
      final n = Node(id: newBlockId(), kind: Kinds.paragraph);
      rec('Add block', () => _push(d.insertFirstChild(null, n)));
      focusBlock(n, 0);
    } else {
      focusBlock(d.flat.first, 0);
    }
  }

  void _onTitleChanged(String t) {
    if (t.contains('\n')) {
      final clean = t.replaceAll('\n', '');
      titleCtl.value = TextEditingValue(
        text: clean,
        selection: TextSelection.collapsed(offset: clean.length),
      );
      _focusFirstBlock();
    }
    _renameTimer?.cancel();
    _renameTimer = Timer(
      Debounce.testDelay(const Duration(milliseconds: 300)),
      _saveTitle,
    );
    setState(() {});
  }

  void _saveTitle() {
    _renameTimer?.cancel();
    final d = doc;
    if (d != null && d.title != titleCtl.text) {
      d.title = titleCtl.text;
      session.rename(d.title);
    }
  }

  // ---------------------------------------------------------------------
  // Marks.

  bool markActive(BMarkKind kind) {
    final n = focusedNode;
    if (n == null) return false;
    final sel = ctl(n).selection;
    if (!sel.isValid) return false;
    if (sel.isCollapsed) {
      final p = _pending[kind];
      if (p != null) return p;
      final at = sel.baseOffset;
      return n.marks.any((m) => m.kind == kind && m.start < at && m.end >= at);
    }
    return rangeHas(n.marks, kind, sel.start, sel.end);
  }

  Set<BMarkKind> activeMarks() => {
    for (final k in BMarkKind.values)
      if (markActive(k)) k,
  };

  void toggleMark(
    BMarkKind kind, {
    String url = '',
    TextSelection? range,
    Node? target,
  }) {
    final n = target ?? focusedNode;
    if (n == null || readOnly) return;
    final c = ctl(n);
    final sel = range ?? c.selection;
    if (!sel.isValid) return;
    if (sel.isCollapsed) {
      setState(() => _pending[kind] = !markActive(kind));
      return;
    }
    final on = kind == BMarkKind.link
        ? url.isNotEmpty
        : !rangeHas(n.marks, kind, sel.start, sel.end);
    rec('Format', () {
      n.marks = setMark(n.marks, kind, sel.start, sel.end, on, url: url);
      c.marks = n.marks;
      c.refresh();
      _push([n.setTextOp()]);
    });
    setState(() {});
  }

  /// Colours the selected text (or, with no text selection, the block).
  void colorSelection(String color) {
    final n = focusedNode;
    if (n == null || readOnly) return;
    final c = ctl(n);
    final sel = c.selection;
    if (!sel.isValid || sel.isCollapsed) {
      _colorBlocks([n], color);
      return;
    }
    rec('Color', () {
      n.marks = setColorMark(n.marks, sel.start, sel.end, color);
      c.marks = n.marks;
      c.refresh();
      _push([n.setTextOp()]);
    });
    setState(() {});
  }

  Future<void> openColorMenu(BuildContext anchor) async {
    final n = focusedNode;
    if (n == null) return;
    final sel = ctl(n).selection;
    String? current;
    if (sel.isValid && !sel.isCollapsed) {
      for (final m in n.marks) {
        if (m.kind == BMarkKind.color &&
            m.start <= sel.start &&
            m.end >= sel.end) {
          current = m.url;
        }
      }
    } else {
      current = n.color;
    }
    final String? v;
    if (context.mobile) {
      v = await showSheet<String>(
        context,
        (ctx) => ColorMenu(
          sheet: true,
          current: current,
          onPick: (x) => Navigator.of(ctx).pop(x),
        ),
      );
    } else {
      v = await showPopover<String>(
        context,
        globalRectOf(anchor),
        (ctx) => ColorMenu(
          current: current,
          onPick: (x) => Navigator.of(ctx).pop(x),
        ),
      );
    }
    if (v == null || !mounted || doc?.byId[n.id] == null) return;
    final c = ctl(n);
    if (sel.isValid && !sel.isCollapsed) {
      c.selection = sel;
      focusedId ??= n.id;
    }
    if (sel.isValid && !sel.isCollapsed) {
      rec('Color', () {
        n.marks = setColorMark(n.marks, sel.start, sel.end, v!);
        c.marks = n.marks;
        c.refresh();
        _push([n.setTextOp()]);
      });
      setState(() {});
      focusBlock(n, sel.end);
    } else {
      _colorBlocks([n], v);
      focusBlock(n, sel.isValid ? sel.baseOffset : n.text.length);
    }
  }

  void openLink() {
    final n = focusedNode;
    if (n == null) return;
    final sel = ctl(n).selection;
    if (!sel.isValid || sel.isCollapsed) return;
    final existing = n.marks.where(
      (m) =>
          m.kind == BMarkKind.link && m.start <= sel.start && m.end >= sel.end,
    );
    setState(() {
      linkTarget = (n.id, sel);
      linkAnchor = selRect;
      selRect = null;
      _linkInitial = existing.isEmpty ? '' : existing.first.url;
    });
  }

  String _linkInitial = '';

  void _linkDone(String? url) {
    final t = linkTarget;
    setState(() => linkTarget = null);
    if (t == null || url == null) return;
    final n = doc?.byId[t.$1];
    if (n == null) return;
    toggleMark(BMarkKind.link, url: url, range: t.$2, target: n);
    focusBlock(n, t.$2.end);
  }

  // ---------------------------------------------------------------------
  // Dragging blocks (desktop).

  void dragStart(Node n, Offset global) {
    FocusManager.instance.primaryFocus?.unfocus();
    setState(() {
      dragNode = n;
      if (!selected.contains(n.id)) {
        selected
          ..clear()
          ..add(n.id);
      }
      dragPos = global;
    });
  }

  void dragUpdate(Offset global) {
    final d = doc;
    final n = dragNode;
    final stack = stackKey.currentContext?.findRenderObject() as RenderBox?;
    if (d == null || n == null || stack == null) return;
    int? gap;
    double? y, x;
    Node? beside;
    var right = false;
    Rect? line;
    final geo = _lastGeo;
    final local = stack.globalToLocal(global);
    for (final r in rows.values) {
      if (!r.mounted || r.node.visIndex < 0) continue;
      final box = r.context.findRenderObject() as RenderBox?;
      if (box == null || !box.attached) continue;
      final top = box.localToGlobal(Offset.zero).dy;
      final h = box.size.height;
      if (global.dy >= top && global.dy < top + h) {
        final before = global.dy < top + h / 2;
        gap = r.node.visIndex + (before ? 0 : 1);
        y = stack.globalToLocal(Offset(0, before ? top : top + h)).dy;
        x = r.node.depth * Layout.indent;
        // Near the right edge of a top-level block (or the left edge of
        // a column list): columns.
        if (geo != null && !context.mobile && _canDropBeside(r.node)) {
          final textRight = geo.textLeft + geo.textWidth;
          final edge = math.min(64.0, geo.textWidth / 5);
          final t = stack.globalToLocal(Offset(0, top)).dy;
          if (local.dx > textRight - edge) {
            beside = r.node;
            right = true;
            line = Rect.fromLTWH(textRight + 2, t, 4, h);
          } else if (r.node.kind == Kinds.columnList &&
              local.dx >= geo.textLeft - 2 &&
              local.dx < geo.textLeft + 16) {
            // The left side only adds a column to an existing list: a plain
            // drop just inside the text start stays a move.
            beside = r.node;
            line = Rect.fromLTWH(geo.textLeft - 8, t, 4, h);
          }
        }
        break;
      }
    }
    // Auto-scroll near the edges.
    final vp = listKey.currentContext?.findRenderObject() as RenderBox?;
    if (vp != null && scroll.hasClients) {
      final top = vp.localToGlobal(Offset.zero).dy;
      final bottom = top + vp.size.height;
      if (global.dy < top + 40) {
        scroll.jumpTo(math.max(0, scroll.offset - 12));
      } else if (global.dy > bottom - 40) {
        scroll.jumpTo(
          math.min(scroll.position.maxScrollExtent, scroll.offset + 12),
        );
      }
    }
    setState(() {
      dragPos = stack.globalToLocal(global);
      if (gap != null) {
        dropGap = gap;
        dropY = y;
        dropX = x;
        dropBeside = beside;
        dropRight = right;
        dropSideLine = line;
      }
    });
  }

  /// Whether the dragged blocks can go beside [t]: a top-level block that
  /// is not being dragged itself.
  bool _canDropBeside(Node t) {
    final n = dragNode;
    if (n == null || t.parent != null || readOnly) return false;
    if (t.kind == Kinds.columnList && t.children.isEmpty) return false;
    final tops = selected.length > 1 && selected.contains(n.id)
        ? _selectedTops()
        : [n];
    for (final x in tops) {
      if (x == t || doc!.isInside(t, x)) return false;
    }
    return true;
  }

  /// Puts [tops] in a column beside [t]: a new column list holding [t] and
  /// them, or a new column at the side of [t] when it is a column list.
  /// Columns the blocks leave empty go away.
  void _dropBesideBlock(Node t, List<Node> tops, bool right) {
    final d = doc!;
    final from = {for (final x in tops) x.parent};
    final Node col;
    if (t.kind == Kinds.columnList) {
      col = Node(id: newBlockId(), kind: Kinds.column);
      _push(
        right
            ? d.insertAfter(t.children.last, col)
            : d.insertFirstChild(t, col),
      );
    } else {
      final list = Node(id: newBlockId(), kind: Kinds.columnList);
      final a = Node(id: newBlockId(), kind: Kinds.column)..parent = list;
      final b = Node(id: newBlockId(), kind: Kinds.column)..parent = list;
      list.children.addAll([a, b]);
      _push(d.insertAfter(t, list));
      _push(d.move(t, right ? a : b, null));
      col = right ? b : a;
    }
    Node? after;
    for (final x in tops) {
      _push(d.move(x, col, after));
      after = x;
    }
    _tidyColumns(from);
  }

  /// Removes columns in [parents] that are now empty, and column lists left
  /// with one column (its blocks take the list's place).
  void _tidyColumns(Set<Node?> parents) {
    final d = doc!;
    for (final p in parents) {
      if (p == null ||
          p.kind != Kinds.column ||
          p.children.isNotEmpty ||
          d.byId[p.id] != p) {
        continue;
      }
      final list = p.parent;
      _push(d.delete(p));
      if (list == null ||
          list.kind != Kinds.columnList ||
          d.byId[list.id] != list) {
        continue;
      }
      final cols = [
        for (final c in list.children)
          if (c.kind == Kinds.column) c,
      ];
      if (cols.length > 1) continue;
      var after = list;
      for (final c
          in cols.isEmpty ? const <Node>[] : List.of(cols.first.children)) {
        _push(d.move(c, list.parent, after));
        after = c;
      }
      _push(d.delete(list));
    }
  }

  void dragEnd() {
    final n = dragNode, gap = dropGap, d = doc, beside = dropBeside;
    if (n != null &&
        beside != null &&
        d != null &&
        !readOnly &&
        _canDropBeside(beside)) {
      final tops = selected.length > 1 && selected.contains(n.id)
          ? _selectedTops()
          : [n];
      rec('Columns', () => _dropBesideBlock(beside, tops, dropRight));
    } else if (n != null && gap != null && d != null && !readOnly) {
      final tops = selected.length > 1 && selected.contains(n.id)
          ? _selectedTops()
          : [n];
      rec('Move', () {
        // Drop the first block, then the others after it in order.
        final from = {for (final x in tops) x.parent};
        final ops = d.dropAt(tops.first, gap);
        if (ops == null) return;
        _push(ops);
        var after = tops.first;
        for (final t in tops.skip(1)) {
          if (d.isInside(after, t) || t == after) continue;
          _push(d.move(t, after.parent, after));
          after = t;
        }
        _tidyColumns(from);
      });
    }
    dragCancel();
  }

  void dragCancel() => setState(() {
    dragNode = null;
    dropGap = null;
    dropY = null;
    dropBeside = null;
    dropSideLine = null;
    dragPos = null;
    selected.clear();
  });

  // ---------------------------------------------------------------------
  // Drag-select from the margins (desktop).

  Geo? _lastGeo;

  bool _inMargin(Offset local) {
    final g = _lastGeo;
    if (g == null) return false;
    return local.dx < g.textLeft - 52 ||
        local.dx > g.textLeft + g.textWidth + 12;
  }

  void _marqueeDown(PointerDownEvent e) {
    _pointerDown = true;
    if (context.mobile || e.buttons != kPrimaryButton || doc == null) return;
    final stack = stackKey.currentContext?.findRenderObject() as RenderBox?;
    if (stack == null) return;
    final local = stack.globalToLocal(e.position);
    if (_inMargin(local)) {
      _marqueeStart = local;
      _marqueeEnd = null;
    }
  }

  void _marqueeMove(PointerMoveEvent e) {
    final start = _marqueeStart;
    if (start == null) return;
    final stack = stackKey.currentContext?.findRenderObject() as RenderBox?;
    if (stack == null) return;
    final local = stack.globalToLocal(e.position);
    if (!_marquee && (local - start).distance < 6) return;
    if (!_marquee) {
      _marquee = true;
      FocusManager.instance.primaryFocus?.unfocus();
    }
    final rect = Rect.fromPoints(start, local);
    final hit = <String>{};
    for (final r in rows.values) {
      if (!r.mounted) continue;
      final box = r.context.findRenderObject() as RenderBox?;
      if (box == null || !box.attached) continue;
      final tl = stack.globalToLocal(box.localToGlobal(Offset.zero));
      final rr = tl & box.size;
      if (rr.top < rect.bottom && rr.bottom > rect.top) hit.add(r.node.id);
    }
    setState(() {
      _marqueeEnd = local;
      selected
        ..clear()
        ..addAll(hit);
      if (hit.isNotEmpty) {
        _selAnchor = doc!.flat.firstWhere((n) => hit.contains(n.id)).id;
      }
    });
  }

  void _marqueeUp(PointerUpEvent e) {
    _pointerDown = false;
    final was = _marquee;
    final start = _marqueeStart;
    _marqueeStart = null;
    _marquee = false;
    if (was) {
      setState(() => _marqueeEnd = null);
      if (selected.isNotEmpty) _pageFocus.requestFocus();
    } else if (start != null && selected.isNotEmpty && menuBlockId == null) {
      setState(() => selected.clear());
    }
    if (!context.mobile) {
      SchedulerBinding.instance.addPostFrameCallback(
        (_) => _updateSelectionToolbar(),
      );
    }
  }

  // ---------------------------------------------------------------------
  // Page actions.

  bool get _favorite => _page?.favorite ?? false;

  Future<void> _toggleFavorite() async {
    final p = _page;
    if (p == null) return;
    final on = !p.favorite;
    setState(() => _page = _withMeta(p, favorite: on));
    await context.app.vault.setFavorite(widget.pageId, on);
  }

  Future<void> _setIcon(String? icon) async {
    final p = _page;
    if (p == null) return;
    setState(() => _page = _withMeta(p, icon: icon, clearIcon: icon == null));
    await context.app.vault.setIcon(widget.pageId, icon);
  }

  Future<void> _setCover(String? cover) async {
    final p = _page;
    if (p == null) return;
    setState(
      () => _page = _withMeta(p, cover: cover, clearCover: cover == null),
    );
    await context.app.vault.run(
      (v) => v.setCover(id: widget.pageId, cover: cover),
      writes: true,
    );
  }

  static PageData _withMeta(
    PageData p, {
    bool? favorite,
    String? icon,
    bool clearIcon = false,
    String? cover,
    bool clearCover = false,
  }) => PageData(
    id: p.id,
    title: p.title,
    blocks: p.blocks,
    parent: p.parent,
    icon: clearIcon ? null : (icon ?? p.icon),
    cover: clearCover ? null : (cover ?? p.cover),
    color: p.color,
    role: p.role,
    properties: p.properties,
    pageType: p.pageType,
    favorite: favorite ?? p.favorite,
    inTrash: p.inTrash,
    createdMs: p.createdMs,
    updatedMs: p.updatedMs,
    prefs: p.prefs,
    loadUs: p.loadUs,
  );

  /// The page's "•••" menu: a popover on desktop, a sheet on mobile.
  Future<void> _openPageMenu(BuildContext anchor) async {
    FocusManager.instance.primaryFocus?.unfocus();
    final rect = context.mobile ? null : globalRectOf(anchor);
    final a = await showPageMenu(
      context,
      rect,
      pageId: widget.pageId,
      favorite: _favorite,
      canUndo: canUndo,
      canAskAi: !readOnly,
      // The hub refuses to share databases, so they get no Share item.
      canShare:
          (_page?.pageType ?? documentType) != databaseType &&
          defaultShareClient(context) != null,
    );
    if (a == null || !mounted) return;
    switch (a) {
      case PageMenuAction.favorite:
        await _toggleFavorite();
      case PageMenuAction.copyLink:
        await Clipboard.setData(
          ClipboardData(text: PageLink(widget.pageId).url),
        );
      case PageMenuAction.share:
        _saveTitle();
        await session.save();
        if (!mounted) return;
        await showSharePopup(
          context,
          pageId: widget.pageId,
          pageTitle: titleCtl.text,
        );
      case PageMenuAction.duplicate:
        _saveTitle();
        await session.save();
        if (!mounted) return;
        final id = await context.app.vault.duplicatePage(widget.pageId);
        if (!mounted) return;
        navOpenPage(context, id, titleCtl.text, stack: true);
      case PageMenuAction.moveTo:
        await showMoveTo(context, widget.pageId, anchor: rect);
      case PageMenuAction.askAi:
        await _askAi();
      case PageMenuAction.undo:
        undo();
      case PageMenuAction.history:
        _saveTitle();
        await session.save();
        if (!mounted) return;
        await showVersionHistory(context, widget.pageId);
      case PageMenuAction.trash:
        await showTrash(context, anchor: rect);
      case PageMenuAction.delete:
        await moveToTrash();
      case PageMenuAction.import:
        await showImport(context, parent: widget.pageId);
      case PageMenuAction.export:
        _saveTitle();
        await session.save();
        if (!mounted) return;
        await showExport(context, widget.pageId);
      case PageMenuAction.feedback:
        sendFeedback(context);
    }
  }

  // ---------------------------------------------------------------------
  // Build.

  Geo _geo(double width, bool mobile) {
    if (widget.embedded) {
      final gutter = mobile ? 18.0 : 30.0;
      return Geo(gutter, math.max(0, width - 2 * gutter));
    }
    if (mobile) return Geo(Layout.gutter, width - 2 * Layout.gutter);
    final full = prefs.fullWidth;
    final col = full
        ? width - 2 * 96
        : math.min(Layout.contentWidth, width - 2 * 48);
    final left = (width - col) / 2 + 6;
    return Geo(left, col - 12);
  }

  @override
  Widget build(BuildContext context) {
    _scheduleEditorFocus();
    final mobile = context.mobile;
    final mq = MediaQuery.of(context);
    final inset = mq.viewInsets.bottom;
    final showKb = mobile && focusedId != null && inset > 0;
    final d = doc;
    if (widget.embedded) _scheduleEmbeddedOverlay();

    if (d != null &&
        !_reportedOpen &&
        !widget.navigationPreview &&
        !widget.embedded) {
      _reportedOpen = true;
      Bakeoff.instance.pageBlocksBuilt(d.flat.length, _loadMs);
      // Log the layout once the route has finished sliding in, so the
      // coordinates are where the text rests (measure.ps1 taps them).
      final anim = ModalRoute.of(context)?.animation;
      void logLayout() {
        if (!mounted) return;
        final box = listKey.currentContext?.findRenderObject() as RenderBox?;
        if (box == null) return;
        final dpr = mq.devicePixelRatio;
        final o = box.localToGlobal(Offset.zero);
        final g = _geo(box.size.width, mobile);
        Bakeoff.instance.log(
          'layout page text-x=${((o.dx + g.textLeft + 150) * dpr).round()} '
          'text-y=${((o.dy + box.size.height * 0.45) * dpr).round()} '
          'list-top=${(o.dy * dpr).round()} list-bottom=${((o.dy + box.size.height) * dpr).round()}',
        );
      }

      void afterFrame() =>
          SchedulerBinding.instance.addPostFrameCallback((_) => logLayout());
      if (anim == null || anim.isCompleted) {
        afterFrame();
      } else {
        late final AnimationStatusListener l;
        l = (st) {
          if (st == AnimationStatus.completed) {
            anim.removeStatusListener(l);
            afterFrame();
          }
        };
        anim.addStatusListener(l);
      }
    }

    final body = Focus(
      focusNode: _pageFocus,
      child: Listener(
        onPointerDown: _marqueeDown,
        onPointerMove: _marqueeMove,
        onPointerUp: _marqueeUp,
        onPointerCancel: (_) {
          _pointerDown = false;
          _marqueeStart = null;
          _marquee = false;
        },
        child: Stack(
          key: stackKey,
          children: [
            if (widget.embedded)
              d == null ? const SizedBox.shrink() : _buildList(context, d)
            else
              Positioned.fill(
                child: d == null
                    ? const SizedBox.shrink()
                    : _buildList(context, d),
              ),
            ..._overlays(context, d, inset, showKb),
          ],
        ),
      ),
    );

    if (widget.embedded) {
      final content = Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (!preview) ConflictToplines(pageId: widget.pageId),
          if (readOnly && !preview) _lockedBanner(context),
          body,
          if (mobile && selectMode) _selectBar(context),
        ],
      );
      return OverlayPortal(
        controller: _embeddedOverlay,
        overlayChildBuilder: (_) => mobile
            ? _embeddedMobileOverlay(context)
            : Stack(
                children: _overlays(
                  context,
                  doc,
                  0,
                  false,
                  screenOverlay: true,
                ),
              ),
        child: content,
      );
    }

    final patch = spacePatch(
      headingProgressSpaceColor,
      context.colors.dark ? Brightness.dark : Brightness.light,
    );
    Widget band() => SizedBox(
      key: ValueKey('page-space-band-${widget.pageId}'),
      height: 6,
      width: double.infinity,
      child: ColoredBox(color: patch!.fill),
    );
    final column = Column(
      children: [
        if (patch != null && !mobile) band(),
        if (!preview || widget.navigationPreview)
          mobile ? _mobileBar(context) : _desktopBar(context),
        if (patch != null && mobile) band(),
        if (!preview) ConflictToplines(pageId: widget.pageId),
        if (readOnly && !preview) _lockedBanner(context),
        Expanded(
          child: preview && !widget.navigationPreview
              ? body
              : NotePageBody(
                  args: NoteArgs(
                    pageId: widget.pageId,
                    pageType: _page?.pageType ?? documentType,
                    title: titleCtl.text,
                    editor: body,
                    titleController: titleCtl,
                    onTitleChanged: _onTitleChanged,
                  ),
                ),
        ),
        if (mobile && selectMode) _selectBar(context),
      ],
    );

    if (widget.navigationPreview) return _frame(context, column, inset, false);
    return PopScope(
      onPopInvokedWithResult: (didPop, _) {
        if (didPop) _saveNow();
      },
      child: _frame(context, column, inset, showKb),
    );
  }

  void _scheduleEmbeddedOverlay() {
    if (_embeddedOverlayUpdatePending) return;
    _embeddedOverlayUpdatePending = true;
    SchedulerBinding.instance.addPostFrameCallback((_) {
      _embeddedOverlayUpdatePending = false;
      if (!mounted) return;
      final show =
          (context.mobile &&
              focusedId != null &&
              MediaQuery.viewInsetsOf(context).bottom > 0) ||
          slashId != null ||
          mentionId != null ||
          linkTarget != null ||
          (!context.mobile && (selRect != null || menuBlockId != null));
      if (show && !_embeddedOverlay.isShowing) _embeddedOverlay.show();
      if (!show && _embeddedOverlay.isShowing) _embeddedOverlay.hide();
    });
  }

  void _embeddedScrollChanged() {
    // Screen overlays track the enclosing workspace, not the document's
    // inert shrink-wrap viewport. Re-map their anchors while it scrolls.
    if (_embeddedOverlay.isShowing) _safeSetState();
  }

  /// An embedded document can be taller than its enclosing viewport. Keep
  /// the existing mobile controls above the keyboard, rather than at its end.
  Widget _embeddedMobileOverlay(BuildContext context) {
    final mq = MediaQuery.of(context);
    final inset = mq.viewInsets.bottom;
    final showKb = focusedId != null && inset > 0;
    final bottom = inset + (showKb ? Layout.keyboardToolbarHeight : 0) + 8;
    final maxHeight = math.min(
      5.5 * 44 + 46,
      math.max(120.0, mq.size.height - bottom - 80),
    );
    return Stack(
      children: [
        if (showKb)
          Positioned(
            left: 0,
            right: 0,
            bottom: inset,
            child: TextFieldTapRegion(child: _keyboardToolbar(context)),
          ),
        if (slashId != null)
          Positioned(
            left: 8,
            right: 8,
            bottom: bottom,
            child: TextFieldTapRegion(
              child: SlashMenu(
                key: const ValueKey('slash-menu'),
                types: _slashTypes,
                active: slashIndex,
                onPick: chooseSlash,
                onHover: (i) => setState(() => slashIndex = i),
                title: slashTurnInto ? 'Turn into' : 'Basic blocks',
                grouped: !slashTurnInto,
                maxHeight: maxHeight,
              ),
            ),
          ),
        if (mentionId != null)
          Positioned(
            left: 8,
            right: 8,
            bottom: bottom,
            child: TextFieldTapRegion(
              child: MentionMenu(
                key: const ValueKey('mention-menu'),
                items: mentionItems,
                active: mentionIndex,
                onPick: chooseMention,
                onHover: (i) => setState(() => mentionIndex = i),
                maxHeight: maxHeight,
              ),
            ),
          ),
        if (linkTarget != null)
          Positioned(
            left: 8,
            right: 8,
            bottom: bottom,
            child: Center(
              child: LinkBox(initial: _linkInitial, onDone: _linkDone),
            ),
          ),
      ],
    );
  }

  Widget _lockedBanner(BuildContext context) {
    final c = context.colors;
    return Container(
      key: const ValueKey('locked-banner'),
      height: 32,
      alignment: Alignment.center,
      color: c.bgSecondary,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          IconX(Ix.lock, size: 14, color: c.iconSecondary),
          const SizedBox(width: 6),
          Text(
            'Page is locked',
            style: ts(context.type.ui, c.textSecondary, weight: 400),
          ),
          const SizedBox(width: 8),
          TextBtn(
            'Unlock',
            key: const ValueKey('unlock'),
            height: 24,
            color: c.accentText,
            onTap: () {
              _meta.updatePrefs(widget.pageId, (p) => p.locked = false);
            },
          ),
        ],
      ),
    );
  }

  /// Mobile select mode's action bar.
  Widget _selectBar(BuildContext context) {
    final c = context.colors;
    final mq = MediaQuery.of(context);
    final n = selected.isEmpty ? null : doc?.byId[selected.first];
    Widget btn(String key, Widget icon, String label, VoidCallback? onTap) =>
        Expanded(
          child: Hover(
            key: ValueKey('select-$key'),
            onTap: onTap,
            radius: Radii.md,
            child: SizedBox(
              height: 52,
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  icon,
                  const SizedBox(height: 2),
                  Text(
                    label,
                    style: tsRaw(
                      11,
                      14,
                      500,
                      onTap == null ? c.textDisabled : c.textSecondary,
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
    final has = selected.isNotEmpty;
    return Container(
      key: const ValueKey('select-bar'),
      padding: EdgeInsets.only(bottom: mq.viewInsets.bottom > 0 ? 0 : 0),
      decoration: BoxDecoration(
        color: c.bgElevated,
        border: Border(top: BorderSide(color: c.borderSubtle)),
      ),
      child: Row(
        children: [
          btn(
            'delete',
            Icon20(Ico.trash, size: 22, color: c.iconSecondary),
            'Delete',
            has ? () => _deleteBlocks(_selectedTops()) : null,
          ),
          btn(
            'duplicate',
            Icon20(Ico.duplicate, size: 22, color: c.iconSecondary),
            'Duplicate',
            has ? () => _duplicate(_selectedTops()) : null,
          ),
          btn(
            'turn-into',
            Icon20(Ico.turnInto, size: 22, color: c.iconSecondary),
            'Turn into',
            n == null
                ? null
                : () async {
                    final r = await showSheet<SheetResult>(
                      context,
                      (_) => _TurnIntoSheet(kind: n.kind),
                    );
                    if (r != null && mounted) {
                      blockAction(n, BlockAction.turnInto, r.kind);
                    }
                  },
          ),
          btn(
            'color',
            IconX(Ix.color, size: 22, color: c.iconSecondary),
            'Color',
            n == null
                ? null
                : () async {
                    final v = await showSheet<String>(
                      context,
                      (ctx) => ColorMenu(
                        sheet: true,
                        onPick: (x) => Navigator.of(ctx).pop(x),
                      ),
                    );
                    if (v != null && mounted) _colorBlocks(_selectedAll(), v);
                  },
          ),
          btn(
            'up',
            Icon20(Ico.arrowUp, size: 22, color: c.iconSecondary),
            'Up',
            has ? () => _moveBlocks(_selectedTops(), -1) : null,
          ),
          btn(
            'down',
            Icon20(Ico.arrowDown, size: 22, color: c.iconSecondary),
            'Down',
            has ? () => _moveBlocks(_selectedTops(), 1) : null,
          ),
        ],
      ),
    );
  }

  Widget _frame(
    BuildContext context,
    Widget column,
    double inset,
    bool showKb,
  ) {
    final c = context.colors;
    final mobile = context.mobile;
    final mq = MediaQuery.of(context);
    if (preview && !widget.navigationPreview) {
      return ColoredBox(color: c.bgPrimary, child: column);
    }
    if (mobile) {
      return ColoredBox(
        color: c.bgPrimary,
        child: Padding(
          padding: EdgeInsets.only(top: mq.padding.top),
          child: Stack(
            children: [
              Padding(
                padding: EdgeInsets.only(
                  bottom: inset > 0
                      ? inset + (showKb ? Layout.keyboardToolbarHeight : 0)
                      : mq.padding.bottom,
                ),
                child: column,
              ),
              if (showKb)
                Positioned(
                  left: 0,
                  right: 0,
                  bottom: inset,
                  child: TextFieldTapRegion(child: _keyboardToolbar(context)),
                ),
            ],
          ),
        ),
      );
    }
    final sheets = PageStackScope.of(context);
    final index = sheets.indexOf(widget.pageId);
    // A popping sheet has left the stack already; it keeps its place.
    if (index > 0) {
      _sheetIndex = index;
      _sheetLeft = PileGeometry.sheetLeft(index, sheets.pages.length);
    }
    final covered = sheets.topId != widget.pageId;
    final route = ModalRoute.of(context)!;
    return Row(
      children: [
        Sidebar(currentPageId: widget.pageId, currentTitle: titleCtl.text),
        Expanded(
          child: LayoutBuilder(
            builder: (context, box) {
              final left = _sheetLeft;
              // The covered editor keeps its own constraints and scroll position.
              // A sheet moving in the pile does not reflow its paragraphs.
              _sheetWidth ??= math.max(0, box.maxWidth - left);
              return Stack(
                clipBehavior: Clip.none,
                children: [
                  // The bottom sheet lays the desk over Home.
                  if (_sheetIndex == 1)
                    Positioned.fill(
                      child: IgnorePointer(
                        child: FadeTransition(
                          opacity: route.animation!,
                          child: ColoredBox(
                            key: const ValueKey('stack-desk'),
                            color: c.desk,
                          ),
                        ),
                      ),
                    ),
                  AnimatedPositioned(
                    duration: Motion.of(context).sheet,
                    curve: Motion.ease,
                    left: left,
                    top: PileGeometry.margin,
                    right: 0,
                    bottom: 0,
                    child: SlideTransition(
                      position:
                          Tween(
                            begin: const Offset(1, 0),
                            end: Offset.zero,
                          ).animate(
                            CurvedAnimation(
                              parent: route.animation!,
                              curve: Motion.ease,
                            ),
                          ),
                      child: Container(
                        key: ValueKey('stack-sheet-${widget.pageId}'),
                        decoration: BoxDecoration(
                          color: c.bgPrimary,
                          borderRadius: const BorderRadius.only(
                            topLeft: Radius.circular(16),
                          ),
                          boxShadow: c.stack,
                        ),
                        child: ClipRRect(
                          borderRadius: const BorderRadius.only(
                            topLeft: Radius.circular(16),
                          ),
                          child: LayoutBuilder(
                            builder: (context, sheetBox) {
                              if (!covered) _sheetWidth = sheetBox.maxWidth;
                              return SheetCover(
                                covered: covered,
                                animation: route.secondaryAnimation,
                                child: OverflowBox(
                                  alignment: Alignment.topLeft,
                                  minWidth: _sheetWidth,
                                  maxWidth: _sheetWidth,
                                  child: column,
                                ),
                              );
                            },
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              );
            },
          ),
        ),
      ],
    );
  }

  Widget _buildList(BuildContext context, Doc d) {
    final mobile = context.mobile;
    return LayoutBuilder(
      builder: (context, box) {
        final geo = _geo(box.maxWidth, mobile);
        _lastGeo = geo;
        // One empty state: starters under the title, the cat below the body.
        final empty =
            _emptyPage &&
            !widget.embedded &&
            !preview &&
            !readOnly &&
            (noteTypeRegistry.of(_page?.pageType ?? documentType)?.usesEditor ??
                true);
        return NotificationListener<ScrollNotification>(
          onNotification: (s) {
            if (s is ScrollStartNotification) {
              Bakeoff.instance.scrollStarted('page');
            }
            if (s is ScrollEndNotification) Bakeoff.instance.scrollEnded();
            if (s.metrics.axis == Axis.vertical) {
              final sc = s.metrics.pixels > 0;
              if (sc != _scrolled) setState(() => _scrolled = sc);
            }
            return false;
          },
          child: CustomScrollView(
            key: listKey,
            controller: scroll,
            shrinkWrap: widget.embedded,
            physics: widget.embedded
                ? const NeverScrollableScrollPhysics()
                : null,
            slivers: [
              if (!widget.embedded)
                SliverToBoxAdapter(child: _header(context, geo, box.maxWidth)),
              if (!widget.embedded)
                SliverToBoxAdapter(
                  child: PageProperties(
                    pageId: widget.pageId,
                    parent: _page?.parent,
                    left: geo.textLeft,
                    width: geo.textWidth,
                  ),
                ),
              if (empty)
                SliverToBoxAdapter(
                  child: PageStarters(
                    key: ValueKey('starters-${widget.pageId}'),
                    pageId: widget.pageId,
                    parent: _page?.parent,
                    left: geo.textLeft,
                    width: geo.textWidth,
                    onBlank: _focusFirstBlock,
                  ),
                ),
              SliverList(
                delegate: SliverChildBuilderDelegate(
                  (context, i) {
                    final n = d.visible[i];
                    return BlockRow(
                      key: ValueKey(n.id),
                      node: n,
                      ed: this,
                      geo: geo,
                    );
                  },
                  childCount: d.visible.length,
                  findChildIndexCallback: (key) {
                    final id = (key as ValueKey<String>).value;
                    final i = d.byId[id]?.visIndex;
                    return i == null || i < 0 ? null : i;
                  },
                ),
              ),
              if (empty)
                SliverToBoxAdapter(
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: _focusFirstBlock,
                    child: Padding(
                      padding: EdgeInsets.only(
                        left: geo.textLeft,
                        top: 8,
                        right: 24,
                      ),
                      child: IgnorePointer(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            SleepingCatLoaf(
                              key: const ValueKey('empty-page-cat'),
                              patch:
                                  spacePatch(
                                    headingProgressSpaceColor,
                                    context.colors.dark
                                        ? Brightness.dark
                                        : Brightness.light,
                                  ) ??
                                  spacePatch(
                                    'apricot',
                                    context.colors.dark
                                        ? Brightness.dark
                                        : Brightness.light,
                                  )!,
                            ),
                            const SizedBox(height: 10),
                            Text(
                              'Nothing here yet. Start typing, or press / for blocks.',
                              style: ts(
                                context.type.body,
                                context.colors.textSecondary,
                                weight: 400,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              if (!preview && !widget.embedded)
                SliverToBoxAdapter(
                  child: BacklinksBar(
                    pageId: widget.pageId,
                    parent: _page?.parent,
                    left: geo.textLeft,
                    width: geo.textWidth,
                  ),
                ),
              SliverToBoxAdapter(
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTap: _tapBelow,
                  child: SizedBox(
                    height: widget.embedded
                        ? 36
                        : mobile
                        ? 160
                        : math.max(160, box.maxHeight * 0.3),
                  ),
                ),
              ),
            ],
          ),
        );
      },
    );
  }

  void _tapBelow() {
    final d = doc;
    if (d == null || readOnly) return;
    if (selected.isNotEmpty) {
      setState(() => selected.clear());
      return;
    }
    final last = d.roots.isEmpty ? null : d.roots.last;
    if (last != null &&
        last.kind == Kinds.paragraph &&
        last.text.isEmpty &&
        last.children.isEmpty) {
      focusBlock(last, 0);
      return;
    }
    final n = Node(id: newBlockId(), kind: Kinds.paragraph);
    if (d.roots.isEmpty) {
      _push(d.insertFirstChild(null, n));
    } else {
      _push(d.insertAfter(d.roots.last, n));
    }
    setState(() {});
    focusBlock(n, 0);
  }

  Widget _header(BuildContext context, Geo geo, double width) {
    final c = context.colors;
    final ty = context.type;
    final mobile = context.mobile;
    final style = tsDisplay(ty.pageTitle, c.textPrimary);
    final title = Padding(
      padding: EdgeInsets.only(bottom: mobile ? 8 : 12),
      child: Stack(
        children: [
          if (titleCtl.text.isEmpty)
            Positioned.fill(
              child: IgnorePointer(
                child: Text(
                  'Untitled',
                  maxLines: 1,
                  style: style.copyWith(color: c.textTertiary),
                ),
              ),
            ),
          EditableText(
            key: const ValueKey('page-title'),
            controller: titleCtl,
            focusNode: titleFocus,
            readOnly: readOnly,
            style: style,
            cursorColor: c.textPrimary,
            backgroundCursorColor: c.textTertiary,
            selectionColor: c.textSelection,
            cursorWidth: 1.5,
            maxLines: null,
            keyboardType: TextInputType.text,
            textInputAction: TextInputAction.next,
            selectionControls: noteSelectionControls,
            onChanged: _onTitleChanged,
            onSubmitted: (_) => _focusFirstBlock(),
          ),
        ],
      ),
    );
    return Padding(
      padding: EdgeInsets.only(
        top: _page?.cover != null ? 0 : (mobile ? 8 : 0),
      ),
      child: PageHeader(
        pageId: widget.pageId,
        patch: spacePatch(
          headingProgressSpaceColor,
          c.dark ? Brightness.dark : Brightness.light,
        ),
        icon: _page?.icon,
        cover: _page?.cover,
        textLeft: geo.textLeft,
        textWidth: geo.textWidth,
        width: width,
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            title,
            if (_page != null)
              NoteTitleChips(
                page: _page!,
                providers: appAddonContext.titleChips,
              ),
          ],
        ),
        readOnly: readOnly,
        onIcon: _setIcon,
        onCover: _setCover,
      ),
    );
  }

  Widget _moreButton({double size = 28, double glyph = 20}) => Builder(
    builder: (b) => IconBtn(
      Ico.more,
      key: const ValueKey('page-menu'),
      onTap: () => _openPageMenu(b),
      size: size,
      glyph: glyph,
      color: b.colors.iconPrimary,
      semanticLabel: 'More',
    ),
  );

  Widget _desktopBar(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return SizedBox(
      height: Layout.topbarHeight,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12),
        child: Row(
          children: [
            const SidebarReopenButton(),
            const CollapsedFeedbackButton(),
            Expanded(
              child: Align(
                alignment: Alignment.centerLeft,
                child: Breadcrumbs(
                  pageId: widget.pageId,
                  title: titleCtl.text,
                  icon: _page?.icon,
                ),
              ),
            ),
            const SyncIndicator(showLabel: false),
            if (_page != null)
              Text(
                'Edited ${relativeTime(_page!.updatedMs.toInt()).replaceFirst('Just now', 'just now')}',
                style: ts(ty.ui, c.textTertiary, weight: 400),
              ),
            const SizedBox(width: 8),
            IconBtn(
              Ico.star,
              key: const ValueKey('page-favorite'),
              onTap: _toggleFavorite,
              selected: _favorite,
              color: _favorite ? c.accent : c.iconPrimary,
              semanticLabel: 'Favorite',
            ),
            const SizedBox(width: 2),
            const AddonHeaderActions(),
            _moreButton(),
            IconBtn(
              Ico.close,
              key: const ValueKey('back'),
              onTap: () => goHome(context),
              color: c.iconSecondary,
              semanticLabel: 'Close page and return Home',
            ),
          ],
        ),
      ),
    );
  }

  Widget _mobileBar(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final label = pageLabelOf(context, widget.pageId, titleCtl.text);
    final titleStyle = ts(ty.ui, c.textPrimary);
    return Container(
      height: Layout.topbarHeight,
      decoration: BoxDecoration(
        color: c.bgPrimary,
        border: Border(
          bottom: BorderSide(
            color: _scrolled ? c.borderSubtle : const Color(0x00000000),
          ),
        ),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 4),
      child: Row(
        children: [
          IconBtn(
            Ico.chevronLeft,
            key: const ValueKey('back'),
            onTap: () {
              final history = MobilePageHistoryScope.maybeOf(context);
              if (history != null) {
                unawaited(history.back());
              } else {
                Navigator.of(context).maybePop();
              }
            },
            size: 44,
            glyph: 24,
            color: c.iconPrimary,
            semanticLabel: 'Back',
          ),
          SpaceIconTile(
            icon: _page?.icon,
            size: 22,
            glyphSize: 18,
            patch: spacePatch(
              headingProgressSpaceColor,
              c.dark ? Brightness.dark : Brightness.light,
            ),
            color: c.iconSecondary,
          ),
          const SizedBox(width: 6),
          Expanded(
            child: Text(
              label.text,
              key: const ValueKey('mobile-title'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: label.firstLine
                  ? firstLineStyle(titleStyle, c.textTertiary)
                  : titleStyle,
            ),
          ),
          if (selectMode)
            TextBtn(
              'Done',
              key: const ValueKey('select-done-top'),
              color: c.accentText,
              onTap: () {
                setState(() {
                  selectMode = false;
                  selected.clear();
                });
              },
            )
          else ...[
            IconBtn(
              Ico.star,
              key: const ValueKey('page-favorite'),
              onTap: _toggleFavorite,
              size: 44,
              glyph: 22,
              color: _favorite ? c.accent : c.iconPrimary,
              semanticLabel: _favorite ? 'Remove from favorites' : 'Favorite',
            ),
            _moreButton(size: 44, glyph: 22),
            const AddonHeaderActions(),
          ],
        ],
      ),
    );
  }

  Widget _keyboardToolbar(BuildContext context) {
    final n = focusedNode;
    return Builder(
      builder: (anchor) => KeyboardToolbar(
        mode: kbMode,
        active: activeMarks(),
        kind: n?.kind ?? Kinds.paragraph,
        canUndo: canUndo,
        canRedo: canRedo,
        onAdd: () {
          if (n != null) addBlockBelow(n);
        },
        onMode: (m) => setState(() => kbMode = m),
        onTurnInto: openTurnIntoMenu,
        onKind: (k) {
          if (n != null) blockTurnInto(n, n.kind == k ? Kinds.paragraph : k);
        },
        onIndent: () {
          if (n != null) indent(n);
        },
        onOutdent: () {
          if (n != null) outdent(n);
        },
        onUndo: undo,
        onRedo: redo,
        onHide: () => FocusManager.instance.primaryFocus?.unfocus(),
        onMark: toggleMark,
        onLink: openLink,
        onMention: startMention,
        onColor: () => openColorMenu(anchor),
      ),
    );
  }

  /// The mobile `/` menu sits above the keyboard toolbar and stops short of
  /// the caret line.
  double _mobileMenuHeight(Size size, Rect? a) {
    final free = a == null
        ? size.height * 0.5
        : size.height - 8 - (a.bottom + 8);
    return clampD(free, 3 * 44 + 46, 5.5 * 44 + 46);
  }

  List<Widget> _overlays(
    BuildContext context,
    Doc? d,
    double inset,
    bool showKb, {
    bool screenOverlay = false,
  }) {
    if (d == null) return const [];
    final mobile = context.mobile;
    final c = context.colors;
    final out = <Widget>[];
    final stackBox = stackKey.currentContext?.findRenderObject() as RenderBox?;
    final overlayBox = screenOverlay
        ? Overlay.of(context).context.findRenderObject() as RenderBox?
        : null;
    final size = screenOverlay
        ? (overlayBox?.size ?? MediaQuery.of(context).size)
        : stackBox?.hasSize == true
        ? stackBox!.size
        : MediaQuery.of(context).size;
    Rect? menuRect(Rect? rect) {
      if (rect == null ||
          !screenOverlay ||
          stackBox == null ||
          overlayBox == null) {
        return rect;
      }
      return Rect.fromPoints(
        overlayBox.globalToLocal(stackBox.localToGlobal(rect.topLeft)),
        overlayBox.globalToLocal(stackBox.localToGlobal(rect.bottomRight)),
      );
    }

    double menuHeight(Rect? anchor, double maximum) {
      if (!screenOverlay || anchor == null) return maximum;
      final below = size.height - anchor.bottom - 8;
      final above = anchor.top - 8;
      final available = below < 240 && anchor.top > below ? above : below;
      return math.min(maximum, math.max(48.0, available - 6));
    }

    final menus = !widget.embedded || screenOverlay;

    // Drag-select rectangle.
    if (!screenOverlay &&
        _marquee &&
        _marqueeStart != null &&
        _marqueeEnd != null) {
      final r = Rect.fromPoints(_marqueeStart!, _marqueeEnd!);
      out.add(
        Positioned.fromRect(
          rect: r,
          child: IgnorePointer(
            child: Container(
              key: const ValueKey('marquee'),
              decoration: BoxDecoration(
                color: c.accent.withValues(alpha: 0.08),
                border: Border.all(color: c.accent.withValues(alpha: 0.3)),
              ),
            ),
          ),
        ),
      );
    }

    // Drop line and drag ghost.
    final side = dropSideLine;
    if (!screenOverlay &&
        dragNode != null &&
        dropBeside != null &&
        side != null) {
      out.add(
        Positioned.fromRect(
          rect: side,
          child: IgnorePointer(
            child: Container(color: c.accent.withValues(alpha: 0.45)),
          ),
        ),
      );
    } else if (!screenOverlay && dragNode != null && dropY != null) {
      final geo = _geo(size.width, mobile);
      out.add(
        Positioned(
          left: geo.textLeft + (dropX ?? 0) - 2,
          top: dropY! - 2,
          width: geo.textWidth - (dropX ?? 0) + 4,
          height: 4,
          child: IgnorePointer(
            child: Container(color: c.accent.withValues(alpha: 0.45)),
          ),
        ),
      );
    }
    if (!screenOverlay && dragNode != null && dragPos != null) {
      final count = selected.length > 1 && selected.contains(dragNode!.id)
          ? _selectedTops().length
          : 1;
      out.add(
        Positioned(
          left: dragPos!.dx + 12,
          top: dragPos!.dy - 12,
          child: IgnorePointer(
            child: Opacity(
              opacity: 0.4,
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 360),
                child: Text(
                  count > 1
                      ? '$count blocks'
                      : (dragNode!.text.isEmpty
                            ? blockRegistry.of(dragNode!.kind).label
                            : dragNode!.text),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: ts(context.type.body, c.textPrimary),
                ),
              ),
            ),
          ),
        ),
      );
    }

    // `/` menu.
    if (menus && slashId != null) {
      final types = _slashTypes;
      final menu = TextFieldTapRegion(
        child: SlashMenu(
          key: const ValueKey('slash-menu'),
          types: types,
          active: slashIndex,
          onPick: chooseSlash,
          onHover: (i) => setState(() => slashIndex = i),
          title: slashTurnInto ? 'Turn into' : 'Basic blocks',
          grouped: !slashTurnInto,
          maxHeight: mobile
              ? _mobileMenuHeight(size, slashAnchor)
              : menuHeight(menuRect(slashAnchor), 400),
        ),
      );
      out.add(_placeMenu(menu, menuRect(slashAnchor), size, mobile));
    }

    // `@` menu.
    if (menus && mentionId != null) {
      final menu = TextFieldTapRegion(
        child: MentionMenu(
          key: const ValueKey('mention-menu'),
          items: mentionItems,
          active: mentionIndex,
          onPick: chooseMention,
          onHover: (i) => setState(() => mentionIndex = i),
          maxHeight: mobile
              ? _mobileMenuHeight(size, mentionAnchor)
              : menuHeight(menuRect(mentionAnchor), 360),
        ),
      );
      out.add(_placeMenu(menu, menuRect(mentionAnchor), size, mobile));
    }

    // Selection toolbar (desktop).
    final n = focusedNode;
    if (menus &&
        !mobile &&
        selRect != null &&
        n != null &&
        slashId == null &&
        mentionId == null &&
        linkTarget == null &&
        !readOnly) {
      final r = menuRect(selRect)!;
      final top = r.top - 8 - SelectionToolbar.height;
      final desiredTop = top < 4 ? r.bottom + 8 : top;
      out.add(
        Positioned(
          left: clampD(r.left, 8, math.max(8, size.width - 360)),
          top: screenOverlay
              ? clampD(
                  desiredTop,
                  8,
                  math.max(8, size.height - SelectionToolbar.height - 8),
                )
              : desiredTop,
          child: TextFieldTapRegion(
            child: SelectionToolbar(
              key: const ValueKey('selection-toolbar'),
              kind: n.kind,
              active: activeMarks(),
              onMark: toggleMark,
              onLink: openLink,
              onTurnInto: openTurnIntoMenu,
              onColor: openColorMenu,
              onAskAgent: _canAskAgent ? askAgentForSelection : null,
              onAskAi: readOnly ? null : askAiForSelection,
            ),
          ),
        ),
      );
    }

    // Link box.
    if (menus && linkTarget != null) {
      final box = LinkBox(initial: _linkInitial, onDone: _linkDone);
      if (mobile) {
        out.add(
          Positioned(left: 8, right: 8, bottom: 8, child: Center(child: box)),
        );
      } else {
        final a = menuRect(linkAnchor) ?? const Rect.fromLTWH(100, 100, 10, 10);
        out.add(
          Positioned(
            left: clampD(a.left, 8, math.max(8, size.width - 330)),
            top: screenOverlay
                ? clampD(a.bottom + 8, 8, math.max(8, size.height - 56))
                : a.bottom + 8,
            child: box,
          ),
        );
      }
    }

    // Block menu (desktop).
    if (menus && menuBlockId != null && menuAnchor != null) {
      final mn = d.byId[menuBlockId];
      if (mn != null) {
        final a = menuRect(menuAnchor)!;
        out.add(
          Positioned.fill(
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: closeBlockMenu,
            ),
          ),
        );
        final top = math.min(
          a.bottom + 4,
          size.height - (screenOverlay ? 408 : 330),
        );
        final count = selected.contains(mn.id)
            ? math.max(1, _selectedTops().length)
            : 1;
        out.add(
          Positioned(
            left: clampD(a.left, 8, math.max(8, size.width - 330)),
            top: math.max(8, top),
            child: BlockMenu(
              key: const ValueKey('block-menu'),
              kind: mn.kind,
              count: count,
              color: mn.color,
              onAction: (act) => blockAction(mn, act),
              onTurnInto: (k) => blockAction(mn, BlockAction.turnInto, k),
              onColor: (v) => blockAction(mn, BlockAction.color, v),
              onAskAgent: _canAskAgent
                  ? () => askAgentForSelection(block: mn)
                  : null,
              onAskAi: readOnly ? null : () => askAiForSelection(block: mn),
            ),
          ),
        );
      }
    }
    return out;
  }

  Widget _placeMenu(Widget menu, Rect? a, Size size, bool mobile) {
    if (mobile) return Positioned(left: 8, right: 8, bottom: 8, child: menu);
    if (a == null) {
      return const Positioned(left: 0, top: 0, child: SizedBox.shrink());
    }
    final below = size.height - a.bottom - 8;
    final showAbove = below < 240 && a.top > below;
    final left = clampD(a.left - 6, 8, math.max(8, size.width - 324 - 8));
    return showAbove
        ? Positioned(left: left, bottom: size.height - a.top + 6, child: menu)
        : Positioned(left: left, top: a.bottom + 6, child: menu);
  }
}

/// Opens page [id] (the navigation helper; named apart from
/// [PageEditorState.openPage]).
void navOpenPage(
  BuildContext context,
  String id,
  String title, {
  bool isNew = false,
  bool stack = false,
}) => openPage(context, id, title, isNew: isNew, stack: stack);

class _TurnIntoSheet extends StatelessWidget {
  const _TurnIntoSheet({required this.kind});
  final String kind;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return SheetSurface(
      children: [
        const SheetHeader('Turn into'),
        for (final t in blockRegistry.turnIntoTypes)
          SheetItem(
            key: ValueKey('sheet-kind-${t.kind}'),
            leading: KindIcon(t, size: 22, color: c.iconPrimary),
            label: t.label,
            selected: t.kind == kind,
            onTap: () =>
                Navigator.of(context)
                    .pop(SheetResult(BlockAction.turnInto, t.kind)),
          ),
      ],
    );
  }
}
