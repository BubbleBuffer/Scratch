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
import '../../../../ui/database/db_tokens.dart';

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
    final m = context.touch;
    final h = m ? DbList.rowTouch : DbList.row;
    Widget row(int i) {
      final r = src.rowAt(i);
      if (r == null) return SizedBox(height: h);
      return _ListRow(key: ValueKey(r.id), args: a, row: r, height: h);
    }

    final add = AddRowButton(key: const ValueKey('list-new'), label: 'New page', onTap: _add, size: m ? ControlSize.xl : ControlSize.md);
    // Rows reach 6 into the gutter; a view block in the text column has none.
    final side = math.max(0.0, a.gutter - DbList.reach);
    if (a.inline) {
      final shown = math.min(n, _limit);
      return Padding(
        padding: EdgeInsets.only(left: side),
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
      padding: EdgeInsets.fromLTRB(side, 0, side, Db.endPadding),
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
    final m = context.touch;
    final a = args;
    final props = [for (final d in a.store.visibleProps(a.view)) if (shownOnCard(row.valueOf(d))) d];
    return Builder(builder: (cell) {
      editTitleWhenBuilt(cell, a.ctl, row);
      return Hover(
        key: ValueKey('list-${row.title}'),
        onTap: () => a.ctl.openRow(context, row.id, title: row.title),
        radius: Radii.sm,
        padding: const EdgeInsets.symmetric(horizontal: Space.xs),
        child: SizedBox(
          height: height,
          child: Row(children: [
            Flexible(
              flex: 3,
              child: RowTitle(
                row: row,
                style: ts(m ? context.type.body : context.type.ui, c.textPrimary),
                iconSize: (m ? IconSize.lg : IconSize.md).px,
                showIcon: a.view.cfg.showIcon,
              ),
            ),
            const Gap(Space.lg),
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
                          padding: const EdgeInsets.only(left: Space.md),
                          child: ConstrainedBox(
                            constraints: const BoxConstraints(maxWidth: DbList.value),
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
