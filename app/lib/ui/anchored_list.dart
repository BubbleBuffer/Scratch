import 'package:flutter/widgets.dart';

/// Index navigation without laying out the intervening, variable-height rows.
/// The viewport is rebased at the chosen row; history remains scrollable in
/// both directions, and the scroll coordinate is relative to that row.
class AnchoredListController {
  final scroll = ScrollController(keepScrollOffset: false);
  void Function(int)? _jump;
  VoidCallback? _end;
  VoidCallback? _pin;

  void jumpToIndex(int index) => _jump?.call(index);
  void jumpToEnd() => _end?.call();

  /// Makes the top visible row the list's origin, so later changes above or
  /// below it cannot move it on screen. Call before a height change that
  /// affects rows the reader is not looking at.
  void pinTop() => _pin?.call();

  void dispose() => scroll.dispose();
}

class AnchoredList extends StatefulWidget {
  const AnchoredList({
    super.key,
    required this.controller,
    required this.itemCount,
    required this.itemBuilder,
    this.onTopIndexChanged,
    this.onVisibleRangeChanged,
    this.keyOf,
    this.indexOf,
    this.padding = EdgeInsets.zero,
    this.initialIndex,
    this.initialY = 0,
    this.initialGlobalY,
  });

  final AnchoredListController controller;
  final int itemCount;
  final IndexedWidgetBuilder itemBuilder;
  final ValueChanged<int>? onTopIndexChanged;

  /// The first and last visible row indexes, reported when either changes.
  final void Function(int first, int last)? onVisibleRangeChanged;

  /// A stable key per row. With [indexOf], the top visible row stays on
  /// screen when rows are added or removed above it: the list rebases on the
  /// row's new index and keeps its pixel offset. Without both, rows keep their
  /// index.
  final String? Function(int index)? keyOf;
  final int? Function(String key)? indexOf;
  final EdgeInsets padding;

  /// The row to start on, with its top edge at [initialY] in the viewport. It
  /// is read once, when the list is first built.
  final int? initialIndex;
  final double initialY;

  /// An optional screen position for the starting row, measured after the new
  /// viewport lays out. Useful when navigation also changes the header height.
  final double? initialGlobalY;

  @override
  State<AnchoredList> createState() => _AnchoredListState();
}

class _AnchoredListState extends State<AnchoredList> {
  final _forward = GlobalKey();
  final _viewport = GlobalKey();
  final _rows = <int, BuildContext>{};
  int _origin = 0;

  /// The key of the origin row, as of the last time the origin was set.
  String? _originKey;
  int? _top, _visibleFirst, _visibleLast;
  bool _scheduled = false;

  /// The top visible row at the last measure: its key, its index and the y of
  /// its top edge in the viewport.
  String? _anchorKey;
  int _anchorIndex = -1;
  double _anchorY = 0;

  @override
  void initState() {
    super.initState();
    _attach();
    final start = widget.initialIndex;
    if (start != null && start >= 0 && start < widget.itemCount) {
      _setOrigin(start);
      _anchorY = widget.initialY;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || !widget.controller.scroll.hasClients) return;
        final viewport = _viewport.currentContext?.findRenderObject();
        final globalY = widget.initialGlobalY;
        final y = globalY != null && viewport is RenderBox && viewport.hasSize
            ? globalY - viewport.localToGlobal(Offset.zero).dy
            : widget.initialY;
        _anchorY = y;
        widget.controller.scroll.jumpTo(-y);
        _scheduleMeasure();
      });
    } else {
      _originKey = widget.keyOf?.call(_origin);
    }
  }

  void _attach() {
    widget.controller._jump = _jump;
    widget.controller._end = _jumpEnd;
    widget.controller._pin = _pinTop;
    widget.controller.scroll.addListener(_scheduleMeasure);
  }

  /// Sets the origin row. Its key is kept, so a later topology change can
  /// tell that the origin itself moved.
  void _setOrigin(int index) {
    _origin = index;
    _anchorIndex = index;
    _originKey = widget.keyOf?.call(index);
    _anchorKey = _originKey;
  }

  void _pinTop() {
    final index = _anchorIndex;
    if (index < 0 || index >= widget.itemCount) return;
    final y = _anchorY;
    setState(() => _setOrigin(index));
    if (widget.controller.scroll.hasClients) {
      widget.controller.scroll.jumpTo(-y);
    }
  }

  @override
  void didUpdateWidget(AnchoredList oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) {
      oldWidget.controller._jump = null;
      oldWidget.controller._end = null;
      oldWidget.controller._pin = null;
      oldWidget.controller.scroll.removeListener(_scheduleMeasure);
      _attach();
    }
    _origin = _origin.clamp(0, widget.itemCount);
    _rebaseOnKey();
    _scheduleMeasure();
  }

  /// When the top visible row moved to another index, the list rebases on it
  /// and restores the pixel offset it had, once the new rows are laid out.
  void _rebaseOnKey() {
    final key = _anchorKey, find = widget.indexOf, keyOf = widget.keyOf;
    if (key == null || find == null || keyOf == null) return;
    final index = find(key);
    if (index == null) return;
    // The origin moved when its row now sits at another index, or is gone. A
    // change below both leaves the origin in place.
    final originMoved =
        _origin >= widget.itemCount || keyOf(_origin) != _originKey;
    if (index == _anchorIndex && !originMoved) return;
    _setOrigin(index);
    final y = _anchorY;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !widget.controller.scroll.hasClients) return;
      widget.controller.scroll.jumpTo(-y);
      // The measure queued with this rebase ran before the jump took effect;
      // measure again once the jumped layout is in.
      _scheduled = false;
      _scheduleMeasure();
    });
  }

  void _jump(int index) {
    if (widget.itemCount == 0) return;
    setState(() => _setOrigin(index.clamp(0, widget.itemCount - 1)));
    _anchorY = 0;
    if (widget.controller.scroll.hasClients) {
      widget.controller.scroll.jumpTo(0);
    }
    _scheduleMeasure();
  }

  void _jumpEnd() {
    if (widget.itemCount == 0) return;
    _jump(widget.itemCount - 1);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !widget.controller.scroll.hasClients) return;
      final row = _rows[widget.itemCount - 1]?.findRenderObject();
      final viewport = _viewport.currentContext?.findRenderObject();
      if (row is! RenderBox || viewport is! RenderBox) return;
      final position = widget.controller.scroll.position;
      widget.controller.scroll.jumpTo(
        (row.size.height - viewport.size.height).clamp(
          position.minScrollExtent,
          position.maxScrollExtent,
        ),
      );
    });
  }

  void _scheduleMeasure() {
    if (_scheduled) return;
    _scheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _scheduled = false;
      if (!mounted) return;
      final viewport = _viewport.currentContext?.findRenderObject();
      if (viewport is! RenderBox || !viewport.hasSize) return;
      int? first, last;
      double? firstY, lastY;
      for (final entry in _rows.entries) {
        if (!entry.value.mounted) continue;
        final row = entry.value.findRenderObject();
        if (row is! RenderBox || !row.attached || !row.hasSize) continue;
        final y = row.localToGlobal(Offset.zero, ancestor: viewport).dy;
        if (y + row.size.height <= 0 || y >= viewport.size.height) continue;
        if (firstY == null || y < firstY) {
          firstY = y;
          first = entry.key;
        }
        if (lastY == null || y > lastY) {
          lastY = y;
          last = entry.key;
        }
      }
      if (first == null || last == null) return;
      _anchorIndex = first;
      _anchorY = firstY!;
      _anchorKey = widget.keyOf?.call(first);
      if (first != _top) {
        _top = first;
        widget.onTopIndexChanged?.call(first);
      }
      if (first != _visibleFirst || last != _visibleLast) {
        _visibleFirst = first;
        _visibleLast = last;
        widget.onVisibleRangeChanged?.call(first, last);
      }
    });
  }

  Widget _row(BuildContext context, int index) => _MeasuredRow(
    key: ValueKey(widget.keyOf?.call(index) ?? index),
    onMount: (context) {
      _rows[index] = context;
      _scheduleMeasure();
    },
    onUnmount: (context) {
      if (identical(_rows[index], context)) _rows.remove(index);
    },
    child: Padding(
      padding: EdgeInsets.only(
        left: widget.padding.left,
        right: widget.padding.right,
        top: index == 0 ? widget.padding.top : 0,
        bottom: index == widget.itemCount - 1 ? widget.padding.bottom : 0,
      ),
      child: widget.itemBuilder(context, index),
    ),
  );

  /// Reconciles lazy children after insertion/removal. A navigation fallback
  /// (for example a removed item mapping to its group header) is not identity.
  int? _childIndex(Key key, {required bool backwards}) {
    final lookup = widget.indexOf, keyOf = widget.keyOf;
    if (key is! ValueKey<String> || lookup == null || keyOf == null) {
      return null;
    }
    final row = lookup(key.value);
    if (row == null ||
        row < 0 ||
        row >= widget.itemCount ||
        keyOf(row) != key.value) {
      return null;
    }
    final index = backwards ? _origin - row - 1 : row - _origin;
    if (index < 0 ||
        (backwards ? index >= _origin : index >= widget.itemCount - _origin)) {
      return null;
    }
    return index;
  }

  @override
  Widget build(BuildContext context) => SizedBox.expand(
    key: _viewport,
    child: NotificationListener<ScrollMetricsNotification>(
      onNotification: (_) {
        _scheduleMeasure();
        return false;
      },
      child: CustomScrollView(
        controller: widget.controller.scroll,
        center: _forward,
        semanticChildCount: widget.itemCount,
        slivers: [
          SliverList(
            delegate: SliverChildBuilderDelegate(
              (context, index) => _row(context, _origin - index - 1),
              childCount: _origin,
              findChildIndexCallback: (key) =>
                  _childIndex(key, backwards: true),
              semanticIndexCallback: (_, index) => _origin - index - 1,
            ),
          ),
          SliverList(
            key: _forward,
            delegate: SliverChildBuilderDelegate(
              (context, index) => _row(context, _origin + index),
              childCount: widget.itemCount - _origin,
              findChildIndexCallback: (key) =>
                  _childIndex(key, backwards: false),
              semanticIndexOffset: _origin,
            ),
          ),
        ],
      ),
    ),
  );

  @override
  void dispose() {
    widget.controller._jump = null;
    widget.controller._end = null;
    widget.controller._pin = null;
    widget.controller.scroll.removeListener(_scheduleMeasure);
    super.dispose();
  }
}

class _MeasuredRow extends StatefulWidget {
  const _MeasuredRow({
    super.key,
    required this.onMount,
    required this.onUnmount,
    required this.child,
  });
  final ValueChanged<BuildContext> onMount;
  final ValueChanged<BuildContext> onUnmount;
  final Widget child;

  @override
  State<_MeasuredRow> createState() => _MeasuredRowState();
}

class _MeasuredRowState extends State<_MeasuredRow> {
  @override
  void initState() {
    super.initState();
    widget.onMount(context);
  }

  @override
  void didUpdateWidget(_MeasuredRow oldWidget) {
    super.didUpdateWidget(oldWidget);
    oldWidget.onUnmount(context);
    widget.onMount(context);
  }

  @override
  void deactivate() {
    widget.onUnmount(context);
    super.deactivate();
  }

  @override
  void activate() {
    super.activate();
    widget.onMount(context);
  }

  @override
  Widget build(BuildContext context) => widget.child;

  @override
  void dispose() {
    widget.onUnmount(context);
    super.dispose();
  }
}
