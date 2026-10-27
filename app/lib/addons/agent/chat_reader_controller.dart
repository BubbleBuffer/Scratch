// Where the reader is in a chat and how it moves: the plain list's scroll
// position, following the end of a live reply, keeping the message at the top
// of the view when the outline changes, and the jump to a pending reference.
// The outline's own position, folds and jumps are the kit's OutlineController,
// which this composes.

import 'package:agent_kit/agent_kit.dart'
    show ItemRow, OutlineController, OutlineLayout;
import 'package:flutter/widgets.dart';

import '../../src/rust/api/agent.dart' show ChatOutline;
import 'agent_store.dart';

class ChatReaderController {
  ChatReaderController({
    required TickerProvider vsync,
    required this.store,
    required this.presentation,
    required this.mounted,
    required this.update,
  }) : controller = OutlineController(
         vsync: vsync,
         mounted: mounted,
         update: update,
       );

  /// The outline's folds, position, layout and jumps. Its indexed list is
  /// `controller.list`.
  final OutlineController controller;

  /// The chat's store and presentation as they are now, whether the chat is
  /// still on screen, and the chat's `setState`.
  final AgentStore Function() store;
  final AgentPresentation Function() presentation;
  final bool Function() mounted;
  final void Function(VoidCallback) update;

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

  /// Counts the position keeps, so a newer one supersedes an older.
  int _keeps = 0;
  ({String id, double y})? readingAnchor;
  bool _readerMeasureScheduled = false;

  /// Where the indexed list starts after the switch: the outline row of the
  /// plain list's top row, and that row's top edge in the viewport.
  ({int index, double y, double globalY})? restore;

  /// The chat the fold and place state belong to. Another chat resets it.
  String? stateFor;

  int lastMessages = 0;
  String last = '';

  bool _taking = false;

  void dispose() {
    controller.dispose();
    scroll.dispose();
  }

  /// Forgets the place in the last chat: another chat is open.
  void reset() {
    controller.reset();
    indexed = false;
    restore = null;
    plainRows.clear();
    rawOutline = null;
    readingAnchor = null;
    lastMessages = 0;
    last = '';
  }

  /// Whether the reader is at the end of the transcript. Only then does a new
  /// reply follow the end; a reader who sent a message always goes to it.
  bool get atEnd =>
      !controller.list.scroll.hasClients ||
      controller.list.scroll.position.extentAfter < 24;

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
        controller.list.jumpToEnd();
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
        await controller.jump(p.firstMessageId, p.lastMessageId);
      } finally {
        _taking = false;
      }
    });
  }

  /// The plain list's top row, as an index of the outline layout and the y of
  /// its top edge in the viewport. Null when no plain row is on screen.
  ({String key, double y, double globalY})? plainTop() {
    final layout = controller.layout;
    final viewport = plainViewport.currentContext?.findRenderObject();
    if (viewport is! RenderBox || !viewport.hasSize) return null;
    int? best;
    double? bestY;
    for (final entry in plainRows.entries) {
      if (layout == null || entry.key >= layout.rows.length) continue;
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
          'item:${layout.firstAnchorFor(item.itemId) ?? item.itemId.split('#').first}',
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
    final token = ++_keeps;
    final navigated = controller.positionRestore;
    final session = stateFor;
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      // The newly mounted indexed list restores its own origin this frame.
      // Measure after that layout, including the header's actual height.
      if (afterInitialLayout) await WidgetsBinding.instance.endOfFrame;
      bool current() =>
          mounted() &&
          token == _keeps &&
          navigated == controller.positionRestore &&
          session == stateFor &&
          identical(raw, rawOutline) &&
          store().pendingFor(presentation()) == null;
      if (!current()) return;
      var y = _messageY(anchor.id);
      if (y == null) {
        final layout = controller.layout;
        final target = layout?.target(anchor.id, anchor.id);
        final index = target == null
            ? null
            : layout?.rowIndex(OutlineLayout.itemKey(target.itemId));
        if (index == null) return;
        controller.list.jumpToIndex(index);
        await WidgetsBinding.instance.endOfFrame;
        if (!current()) return;
        y = _messageY(anchor.id);
      }
      if (y == null || !controller.list.scroll.hasClients) return;
      final delta = y - anchor.y;
      if (delta.abs() < .5) return;
      final position = controller.list.scroll.position;
      controller.list.scroll.jumpTo(
        (position.pixels + delta).clamp(
          position.minScrollExtent,
          position.maxScrollExtent,
        ),
      );
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }
}
