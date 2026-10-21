// The table view: a sticky header with resizable columns (the title column
// stays in place when scrolling sideways), virtualized rows with inline
// editing, "+ New page", and the calculations footer.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../../../../core/models.dart';
import '../../../../ui/database/config.dart';
import '../../../../ui/database/db_icons.dart';
import '../../../../ui/database/db_theme.dart';
import '../../property_menu.dart';
import '../../../../ui/database/values.dart';
import '../../../../ui/database/widgets.dart';
import '../../../../registry/property_registry.dart';
import '../../../../registry/view_registry.dart';
import '../../../../ui/ui.dart';
import '../common.dart';

final tableView = ViewType(
  kind: ViewKind.table,
  label: 'Table',
  icon: DbIco.table,
  build: (context, a) => TableView(args: a),
  options: (context, a) {
    final cfg = a.view.cfg;
    return [
      ViewOption(
        label: 'Show vertical lines',
        value: Toggle(on: cfg.verticalLines),
        onTap: (_, _) => a.store.updateConfig(
          a.view,
          (c) => c.verticalLines = !c.verticalLines,
        ),
      ),
      ViewOption(
        label: 'Wrap all columns',
        value: Toggle(on: cfg.wrap),
        onTap: (_, _) => a.store.updateConfig(a.view, (c) => c.wrap = !c.wrap),
      ),
      ViewOption(
        label: 'Show page icon',
        value: Toggle(on: cfg.showIcon),
        onTap: (_, _) =>
            a.store.updateConfig(a.view, (c) => c.showIcon = !c.showIcon),
      ),
    ];
  },
);

/// One column: the title (def null) or a property.
class _Col {
  const _Col(this.key, this.def, this.width, this.wrap);
  final String key;
  final PropDef? def;
  final double width;
  final bool wrap;
}

class TableView extends StatefulWidget {
  const TableView({super.key, required this.args});
  final ViewArgs args;

  @override
  State<TableView> createState() => _TableViewState();
}

class _TableViewState extends State<TableView> {
  final _h = ScrollController();
  final _v = ScrollController();
  final _offset = ValueNotifier<double>(0);
  final Map<String, double> _drag = {};
  int _limit = inlinePage;

  ViewArgs get a => widget.args;

  @override
  void initState() {
    super.initState();
    _h.addListener(() => _offset.value = _h.offset);
  }

  @override
  void dispose() {
    _h.dispose();
    _v.dispose();
    _offset.dispose();
    super.dispose();
  }

  List<_Col> _cols(BuildContext context) {
    final v = a.view;
    final cfg = v.cfg;
    final w = cfg.widths;
    final m = context.mobile;
    double width(String key, bool title) =>
        _drag[key] ??
        w[key] ??
        (title ? (m ? 160.0 : Db.titleColumn) : (m ? 140.0 : Db.defaultColumn));
    bool wrap(String key) => cfg.wrap || cfg.wrapped.contains(key);
    return [
      _Col(
        BuiltinKeys.title,
        null,
        width(BuiltinKeys.title, true),
        wrap(BuiltinKeys.title) || m,
      ),
      for (final d in a.store.visibleProps(v))
        _Col(d.key, d, width(d.key, false), wrap(d.key)),
    ];
  }

  double get _rowH => context.mobile ? Db.mobileRowHeight : Db.rowHeight;

  Future<void> _addRow() async {
    final id = await a.ctl.newRow(context, open: false);
    a.ctl.editTitleOf = id;
    if (mounted) setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final cols = _cols(context);
    final frozen = 1 + a.view.cfg.frozen.clamp(0, cols.length - 1);
    final wrapAny = cols.any((c) => c.wrap);
    final g = a.gutter;
    final tableW =
        cols.fold<double>(0, (s, c) => s + c.width) + (context.mobile ? 0 : 34);
    final src = a.store.main;
    final n = math.max(0, src.total);

    Widget header() => _Header(state: this, cols: cols, frozen: frozen);

    Widget row(int i) {
      final r = src.rowAt(i);
      if (r == null) return SizedBox(height: _rowH, width: tableW);
      return _TableRow(
        key: ValueKey(r.id),
        state: this,
        row: r,
        cols: cols,
        frozen: frozen,
        wrapAny: wrapAny,
      );
    }

    final newRow = SizedBox(
      width: tableW,
      child: DecoratedBox(
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: context.colors.gridLine)),
        ),
        child: AddRowButton(
          key: const ValueKey('table-new'),
          label: 'New page',
          onTap: _addRow,
          height: context.mobile ? 44 : 33,
        ),
      ),
    );
    final calcRow = _CalcRow(state: this, cols: cols);

    if (a.inline) {
      final shown = math.min(n, _limit);
      return SingleChildScrollView(
        controller: _h,
        scrollDirection: Axis.horizontal,
        child: Padding(
          padding: EdgeInsets.only(left: g, right: 32),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              header(),
              for (var i = 0; i < shown; i++) row(i),
              if (n > shown)
                LoadMore(onTap: () => setState(() => _limit += inlinePage)),
              newRow,
              calcRow,
            ],
          ),
        ),
      );
    }

    return LayoutBuilder(
      builder: (context, box) {
        final contentW = math.max(box.maxWidth, g + tableW + g);
        return SingleChildScrollView(
          controller: _h,
          scrollDirection: Axis.horizontal,
          child: SizedBox(
            width: contentW,
            height: box.maxHeight,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Padding(
                  padding: EdgeInsets.only(left: g),
                  child: header(),
                ),
                Expanded(
                  child: ListView.builder(
                    key: const ValueKey('table-rows'),
                    controller: _v,
                    // The scroll canvas includes the trailing gutter (and can
                    // be wider than a compact table). Keep row borders within
                    // the same column width as the header.
                    padding: EdgeInsets.only(
                      left: g,
                      right: contentW - g - tableW,
                      bottom: 120,
                    ),
                    itemCount: n + 2,
                    itemExtentBuilder: wrapAny
                        ? null
                        : (i, _) => i < n
                              ? _rowH
                              : (i == n
                                    ? (context.mobile ? 45.0 : 34.0)
                                    : 36.0),
                    itemBuilder: (context, i) {
                      if (i < n) return row(i);
                      if (i == n) return newRow;
                      return calcRow;
                    },
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// Positions the frozen columns so they stay at the left edge while the
/// table scrolls sideways (drawn above the other cells).
Widget _withFrozen(
  _TableViewState s,
  double frozenW,
  Widget rest,
  Widget frozenCells,
) {
  return Stack(
    clipBehavior: Clip.none,
    children: [
      rest,
      Positioned(
        left: 0,
        top: 0,
        bottom: 0,
        width: frozenW,
        child: ValueListenableBuilder<double>(
          valueListenable: s._offset,
          builder: (context, off, child) => Transform.translate(
            offset: Offset(math.max(0, off - s.a.gutter), 0),
            child: child,
          ),
          child: frozenCells,
        ),
      ),
    ],
  );
}

class _Header extends StatelessWidget {
  const _Header({
    required this.state,
    required this.cols,
    required this.frozen,
  });
  final _TableViewState state;
  final List<_Col> cols;
  final int frozen;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final a = state.a;
    final vlines = a.view.cfg.verticalLines;
    final h = context.mobile ? 40.0 : Db.headerHeight;
    Widget cell(int i) => _HeaderCell(
      state: state,
      col: cols[i],
      height: h,
      line: vlines && i < cols.length - 1,
    );
    final frozenW = cols.take(frozen).fold<double>(0, (s, c) => s + c.width);
    final rest = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(width: frozenW, height: h),
        for (var i = frozen; i < cols.length; i++) cell(i),
        if (!context.mobile)
          Builder(
            builder: (b) => Hover(
              key: const ValueKey('table-add-prop'),
              onTap: () => a.ctl.addProperty(b),
              radius: 0,
              child: SizedBox(
                width: 34,
                height: h,
                child: Center(
                  child: Icon20(Ico.plus, size: 16, color: c.textSecondary),
                ),
              ),
            ),
          ),
      ],
    );
    return Container(
      decoration: BoxDecoration(
        color: c.bgSecondary,
        border: Border(
          top: BorderSide(color: c.gridLine),
          bottom: BorderSide(color: c.gridLine),
        ),
      ),
      child: _withFrozen(
        state,
        frozenW,
        rest,
        ColoredBox(
          color: c.bgSecondary,
          child: Row(children: [for (var i = 0; i < frozen; i++) cell(i)]),
        ),
      ),
    );
  }
}

class _HeaderCell extends StatefulWidget {
  const _HeaderCell({
    required this.state,
    required this.col,
    required this.height,
    required this.line,
  });
  final _TableViewState state;
  final _Col col;
  final double height;
  final bool line;

  @override
  State<_HeaderCell> createState() => _HeaderCellState();
}

class _HeaderCellState extends State<_HeaderCell> {
  double _start = 0;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final col = widget.col;
    final s = widget.state;
    final m = context.mobile;
    return SizedBox(
      width: col.width,
      height: widget.height,
      child: Stack(
        children: [
          Positioned.fill(
            child: DecoratedBox(
              decoration: BoxDecoration(
                border: widget.line
                    ? Border(right: BorderSide(color: c.gridLine))
                    : null,
              ),
              child: Builder(
                builder: (b) => Hover(
                  key: ValueKey('col-${col.def?.name ?? 'Name'}'),
                  onTap: () => s.a.ctl.propertyMenu(b, col.def),
                  radius: 0,
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  child: Row(
                    children: [
                      DbIcon(
                        propIcon(col.def),
                        size: m ? 18 : 16,
                        color: c.textSecondary,
                      ),
                      const SizedBox(width: 6),
                      Expanded(
                        child: Text(
                          col.def?.name ?? 'Name',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: tsRaw(13, 20, 600, c.textSecondary),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
          if (!m)
            Positioned(
              right: -3,
              top: 0,
              bottom: 0,
              width: 7,
              child: MouseRegion(
                cursor: SystemMouseCursors.resizeColumn,
                child: GestureDetector(
                  key: ValueKey('resize-${col.def?.name ?? 'Name'}'),
                  behavior: HitTestBehavior.opaque,
                  onHorizontalDragStart: (_) => _start = col.width,
                  onHorizontalDragUpdate: (d) {
                    _start += d.delta.dx;
                    s.setState(
                      () => s._drag[col.key] = math.max(Db.minColumn, _start),
                    );
                  },
                  onHorizontalDragEnd: (_) {
                    final w = s._drag[col.key];
                    if (w == null) return;
                    s.a.store
                        .updateConfig(
                          s.a.view,
                          (c) => c.widths = {
                            ...c.widths,
                            col.key: w.roundToDouble(),
                          },
                        )
                        .then((_) {
                          if (s.mounted) {
                            s.setState(() => s._drag.remove(col.key));
                          }
                        });
                  },
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _TableRow extends StatefulWidget {
  const _TableRow({
    super.key,
    required this.state,
    required this.row,
    required this.cols,
    required this.frozen,
    required this.wrapAny,
  });
  final _TableViewState state;
  final PageRow row;
  final List<_Col> cols;
  final int frozen;
  final bool wrapAny;

  @override
  State<_TableRow> createState() => _TableRowState();
}

class _TableRowState extends State<_TableRow> {
  bool _hover = false;
  String? _activeCell;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final s = widget.state;
    final a = s.a;
    final cols = widget.cols;
    final r = widget.row;
    final vlines = a.view.cfg.verticalLines;
    final h = s._rowH;

    Widget cell(int i) {
      final col = cols[i];
      final line = vlines && i < cols.length - 1;
      final inner = col.def == null
          ? _titleCell(context, col)
          : _valueCell(context, col);
      return Container(
        width: col.width,
        constraints: widget.wrapAny
            ? BoxConstraints(minHeight: h)
            : BoxConstraints.tightFor(height: h),
        foregroundDecoration: _activeCell == col.key
            ? BoxDecoration(border: Border.all(color: c.focus, width: 2))
            : null,
        decoration: BoxDecoration(
          border: line ? Border(right: BorderSide(color: c.gridLine)) : null,
        ),
        child: inner,
      );
    }

    final frozenW = cols
        .take(widget.frozen)
        .fold<double>(0, (s, c) => s + c.width);
    final rest = Row(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        // The frozen cells float above this row (a Stack child that does not
        // size it), so their wrapped height is measured here.
        SizedBox(width: frozenW, height: _frozenHeight(context, h)),
        for (var i = widget.frozen; i < cols.length; i++) cell(i),
        if (!context.mobile) const SizedBox(width: 34),
      ],
    );
    final rowColor = _hover
        ? Color.alphaBlend(c.bgHover, c.bgPrimary)
        : c.bgPrimary;
    final frozenCells = ColoredBox(
      color: rowColor,
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [for (var i = 0; i < widget.frozen; i++) cell(i)],
      ),
    );
    final body = widget.wrapAny
        ? IntrinsicHeight(child: _withFrozen(s, frozenW, rest, frozenCells))
        : SizedBox(
            height: h,
            child: _withFrozen(s, frozenW, rest, frozenCells),
          );
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: Container(
        key: ValueKey('row-${r.title}'),
        decoration: BoxDecoration(
          color: rowColor,
          border: Border(bottom: BorderSide(color: c.gridLine)),
        ),
        child: body,
      ),
    );
  }

  TextStyle _titleStyle(BuildContext context) => ts(
    context.mobile ? const TypeSpec(16, 20, 600) : const TypeSpec(14, 21, 500),
    context.colors.textPrimary,
  );

  /// Height of the frozen cells with wrapped titles (at least [h]).
  double _frozenHeight(BuildContext context, double h) {
    if (!widget.wrapAny) return h;

    final m = context.mobile;
    final a = widget.state.a;
    var best = h;
    for (var i = 0; i < widget.frozen; i++) {
      final col = widget.cols[i];
      if (col.def != null || !col.wrap) continue;

      final icon = a.view.cfg.showIcon ? (m ? 20.0 : 18.0) + 6 : 0.0;
      final w = col.width - 14 - icon - 4 - (m ? 56 : 50);
      final t = displayTitle(widget.row.title, firstLine: untitledLine(context, widget.row));
      final tp = TextPainter(
        text: TextSpan(text: t, style: _titleStyle(context)),
        textDirection: TextDirection.ltr,
        textScaler: MediaQuery.textScalerOf(context),
      )..layout(maxWidth: math.max(24, w));
      best = math.max(best, tp.height + (m ? 13 : 7.5) + 6 + 1);
      tp.dispose();
    }
    return best;
  }

  Widget _titleCell(BuildContext context, _Col col) {
    final s = widget.state;
    final a = s.a;
    final r = widget.row;
    final m = context.mobile;
    final style = _titleStyle(context);
    final showOpen = m || _hover;
    return Builder(
      builder: (cell) {
        editTitleWhenBuilt(cell, a.ctl, r);
        return GestureDetector(
          key: ValueKey('title-${r.title}'),
          behavior: HitTestBehavior.opaque,
          onTap: () async {
            if (m) {
              await a.ctl.openRow(context, r.id, title: r.title);
              return;
            }
            setState(() => _activeCell = col.key);
            try {
              await a.ctl.editTitle(cell, r);
            } finally {
              if (mounted) setState(() => _activeCell = null);
            }
          },
          child: Padding(
            padding: EdgeInsets.fromLTRB(8, m ? 13 : 7.5, 6, 6),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Expanded(
                  child: RowTitle(
                    row: r,
                    style: style,
                    iconSize: m ? 20 : 18,
                    showIcon: a.view.cfg.showIcon,
                    wrap: col.wrap,
                  ),
                ),
                if (showOpen) ...[
                  const SizedBox(width: 4),
                  OpenChip(
                    key: ValueKey('open-${r.title}'),
                    label: 'OPEN',
                    onTap: () => a.ctl.openRow(context, r.id, title: r.title),
                  ),
                ],
              ],
            ),
          ),
        );
      },
    );
  }

  Widget _valueCell(BuildContext context, _Col col) {
    final a = widget.state.a;
    final d = col.def!;
    final t = propertyRegistry.of(d.kind);
    final m = context.mobile;
    return Builder(
      builder: (cell) => GestureDetector(
        key: ValueKey('cell-${widget.row.title}-${d.name}'),
        behavior: HitTestBehavior.opaque,
        onTap: t?.edit == null && t?.tap == null
            ? null
            : () async {
                setState(() => _activeCell = col.key);
                try {
                  await a.ctl.edit(cell, widget.row, d);
                } finally {
                  if (mounted) setState(() => _activeCell = null);
                }
              },
        child: Padding(
          padding: EdgeInsets.fromLTRB(8, m ? 13 : 7.5, 8, 6),
          child: Align(
            alignment: (t?.alignEnd ?? false)
                ? Alignment.topRight
                : Alignment.topLeft,
            child: valueOf(cell, d, widget.row, CellMode.table, wrap: col.wrap),
          ),
        ),
      ),
    );
  }
}

class _CalcRow extends StatelessWidget {
  const _CalcRow({required this.state, required this.cols});
  final _TableViewState state;
  final List<_Col> cols;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 36,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [for (final col in cols) _CalcCell(state: state, col: col)],
      ),
    );
  }
}

class _CalcCell extends StatefulWidget {
  const _CalcCell({required this.state, required this.col});
  final _TableViewState state;
  final _Col col;

  @override
  State<_CalcCell> createState() => _CalcCellState();
}

class _CalcCellState extends State<_CalcCell> {
  bool _hover = false;

  List<Calc> _options() {
    final d = widget.col.def;
    final base = [
      Calc.none,
      Calc.countAll,
      if (d != null) ...[
        Calc.countValues,
        Calc.countEmpty,
        Calc.countNotEmpty,
        Calc.percentEmpty,
        Calc.percentNotEmpty,
      ],
    ];
    if (d == null) return base;

    final t = propertyRegistry.of(d.kind);
    if (d.kind == PropKind.checkbox) {
      return [Calc.none, Calc.countAll, ...?t?.calcs];
    }
    return [...base, ...?t?.calcs];
  }

  Future<void> _pick(BuildContext anchor) async {
    final a = widget.state.a;
    final key = widget.col.key;
    final cur = a.view.cfg.calcs[key] ?? Calc.none;
    final pick = await showPanel<Calc>(
      anchor,
      globalRectOf(anchor),
      (ctx) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final c in _options())
            MenuItem(
              key: ValueKey('calc-${c.name}'),
              label: c.label,
              trailing: c == cur
                  ? Icon20(Ico.check, size: 16, color: ctx.colors.iconPrimary)
                  : null,
              onTap: () => Navigator.of(ctx).pop(c),
            ),
        ],
      ),
      width: 220,
    );
    if (pick == null) return;
    await a.store.updateConfig(a.view, (c) {
      final m = c.calcs;
      if (pick == Calc.none) {
        m.remove(key);
      } else {
        m[key] = pick;
      }
      c.calcs = m;
    });
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final a = widget.state.a;
    final col = widget.col;
    final calc = a.view.cfg.calcs[col.key];
    final value = calc == null ? null : a.store.calc(col.def, col.key, calc);
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: SizedBox(
        width: col.width,
        height: 36,
        child: Builder(
          builder: (b) => Hover(
            key: ValueKey('calc-cell-${col.def?.name ?? 'Name'}'),
            onTap: () => _pick(b),
            radius: 0,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                if (calc != null) ...[
                  Text(
                    calc.short.toUpperCase(),
                    style: tsRaw(
                      10.5,
                      14,
                      500,
                      c.textTertiary,
                    ).copyWith(letterSpacing: 0.6),
                  ),
                  const SizedBox(width: 5),
                  Flexible(
                    child: Text(
                      value ?? '…',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: ts(context.type.ui, c.textSecondary, weight: 400),
                    ),
                  ),
                ] else if (_hover) ...[
                  Text(
                    'Calculate',
                    style: ts(
                      context.type.uiSmall,
                      c.textTertiary,
                      weight: 400,
                    ),
                  ),
                  const SizedBox(width: 2),
                  Icon20(Ico.chevronDown, size: 12, color: c.iconTertiary),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
