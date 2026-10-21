// The list view: one line per row, the title at the left and the shown
// properties right-aligned after it. Virtualized like the table.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../../../../core/models.dart';
import '../../../../ui/database/config.dart';
import '../../cells.dart';
import '../../../../ui/database/db_icons.dart';
import '../../../../ui/database/values.dart';
import '../../../../ui/database/widgets.dart';
import '../../../../registry/property_registry.dart';
import '../../../../registry/view_registry.dart';
import '../../../../ui/ui.dart';
import '../common.dart';

final listView = ViewType(
  kind: ViewKind.list,
  label: 'List',
  icon: DbIco.list,
  build: (context, a) => DbListView(args: a),
  options: (context, a) => [
    ViewOption(
      label: 'Show page icon',
      value: Toggle(on: a.view.cfg.showIcon),
      onTap: (_, _) => a.store.updateConfig(a.view, (c) => c.showIcon = !c.showIcon),
    ),
  ],
);

class DbListView extends StatefulWidget {
  const DbListView({super.key, required this.args});
  final ViewArgs args;

  @override
  State<DbListView> createState() => _DbListViewState();
}

class _DbListViewState extends State<DbListView> {
  int _limit = inlinePage;
  final _v = ScrollController();

  ViewArgs get a => widget.args;

  @override
  void dispose() {
    _v.dispose();
    super.dispose();
  }

  Future<void> _add() async {
    final id = await a.ctl.newRow(context, open: false);
    a.ctl.editTitleOf = id;
    if (mounted) setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final src = a.store.main;
    final n = math.max(0, src.total);
    final m = context.mobile;
    final h = m ? 44.0 : 34.0;
    Widget row(int i) {
      final r = src.rowAt(i);
      if (r == null) return SizedBox(height: h);
      return _ListRow(key: ValueKey(r.id), args: a, row: r, height: h);
    }

    final add = AddRowButton(key: const ValueKey('list-new'), label: 'New page', onTap: _add, height: h - 2);
    if (a.inline) {
      final shown = math.min(n, _limit);
      return Padding(
        padding: EdgeInsets.only(left: a.gutter - 6),
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
          for (var i = 0; i < shown; i++) row(i),
          if (n > shown) LoadMore(onTap: () => setState(() => _limit += inlinePage)),
          add,
        ]),
      );
    }
    return ListView.builder(
      key: const ValueKey('list-rows'),
      controller: _v,
      padding: EdgeInsets.fromLTRB(a.gutter - 6, 0, a.gutter - 6, 120),
      itemCount: n + 1,
      itemExtent: h,
      itemBuilder: (context, i) => i < n ? row(i) : add,
    );
  }
}

class _ListRow extends StatelessWidget {
  const _ListRow({super.key, required this.args, required this.row, required this.height});
  final ViewArgs args;
  final PageRow row;
  final double height;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final a = args;
    final props = [for (final d in a.store.visibleProps(a.view)) if (shownOnCard(row.valueOf(d))) d];
    return Builder(builder: (cell) {
      editTitleWhenBuilt(cell, a.ctl, row);
      return Hover(
        key: ValueKey('list-${row.title}'),
        onTap: () => a.ctl.openRow(context, row.id, title: row.title),
        radius: Radii.sm,
        padding: const EdgeInsets.symmetric(horizontal: 6),
        child: SizedBox(
          height: height,
          child: Row(children: [
            Flexible(
              flex: 3,
              child: RowTitle(
                row: row,
                style: ts(m ? const TypeSpec(16, 22, 500) : const TypeSpec(14, 20, 500), c.textPrimary),
                iconSize: m ? 20 : 18,
                showIcon: a.view.cfg.showIcon,
              ),
            ),
            const SizedBox(width: 16),
            Expanded(
              flex: 4,
              child: ClipRect(
                child: Align(
                  alignment: Alignment.centerRight,
                  child: SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    reverse: true,
                    physics: const NeverScrollableScrollPhysics(),
                    child: Row(mainAxisSize: MainAxisSize.min, children: [
                      for (final d in props)
                        Padding(
                          padding: const EdgeInsets.only(left: 12),
                          child: ConstrainedBox(
                            constraints: const BoxConstraints(maxWidth: 280),
                            child: valueOf(context, d, row, CellMode.list),
                          ),
                        ),
                    ]),
                  ),
                ),
              ),
            ),
          ]),
        ),
      );
    });
  }
}
