// The board view: a column per group of the group-by property (select,
// status, multi-select or checkbox), cards dragged between columns (which
// sets the property) and within a column (which reorders), hidden groups,
// and "+ New" per column. Each column loads and builds only the cards on
// screen.

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../registry/note_type_registry.dart';
import '../../ui/database/config.dart';
import '../../ui/database/db_icons.dart';
import '../../ui/database/db_theme.dart';
import '../database/api.dart';
import '../../ui/database/values.dart';
import '../../ui/database/widgets.dart';
import '../../registry/property_registry.dart';
import '../../registry/view_registry.dart';
import '../../state/collection_store.dart';
import '../../ui/ui.dart';
import '../../ui/board/board_renderer.dart';
import '../../ui/board/board_source.dart';
import 'database_board_source.dart';

final boardView = ViewType(
  kind: ViewKind.board,
  label: 'Board',
  icon: DbIco.board,
  needsGroup: true,
  build: (context, a) => BoardView(args: a),
  options: (context, a) {
    final cfg = a.view.cfg;
    return [
      ViewOption(
        label: 'Card preview',
        value: menuValue(context, _previewLabel(cfg.preview)),
        onTap: (ctx, rect) => _pickPreview(ctx, rect, a),
      ),
      ViewOption(
        label: 'Card size',
        value: menuValue(context, _sizeLabel(cfg.cardSize.name)),
        onTap: (ctx, rect) => _pickSize(ctx, rect, a),
      ),
      ViewOption(
        label: 'Color columns',
        value: Toggle(on: cfg.colorColumns),
        onTap: (_, _) => a.store.updateConfig(
          a.view,
          (c) => c.colorColumns = !c.colorColumns,
        ),
      ),
    ];
  },
);

String _previewLabel(String p) => switch (p) {
  'cover' => 'Page cover',
  'content' => 'Page content',
  _ => 'None',
};
String _sizeLabel(String s) => '${s[0].toUpperCase()}${s.substring(1)}';

Future<void> _pickPreview(BuildContext ctx, Rect rect, ViewArgs a) async {
  final p = await showPanel<String>(
    ctx,
    rect,
    (c) => Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final id in const ['none', 'cover', 'content'])
          MenuItem(
            key: ValueKey('preview-$id'),
            label: _previewLabel(id),
            trailing: a.view.cfg.preview == id
                ? Icon20(Ico.check, size: 16, color: c.colors.iconPrimary)
                : null,
            onTap: () => Navigator.of(c).pop(id),
          ),
      ],
    ),
    width: 220,
  );
  if (p != null) await a.store.updateConfig(a.view, (c) => c.preview = p);
}

Future<void> _pickSize(BuildContext ctx, Rect rect, ViewArgs a) async {
  final s = await showPanel<CardSize>(
    ctx,
    rect,
    (c) => Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final v in CardSize.values)
          MenuItem(
            key: ValueKey('size-${v.name}'),
            label: _sizeLabel(v.name),
            trailing: a.view.cfg.cardSize == v
                ? Icon20(Ico.check, size: 16, color: c.colors.iconPrimary)
                : null,
            onTap: () => Navigator.of(c).pop(v),
          ),
      ],
    ),
    width: 220,
  );
  if (s != null) await a.store.updateConfig(a.view, (c) => c.cardSize = s);
}

class BoardView extends StatefulWidget {
  const BoardView({super.key, required this.args});
  final ViewArgs args;

  @override
  State<BoardView> createState() => _BoardViewState();
}

class _BoardViewState extends State<BoardView> {
  late DatabaseBoardSource _source;

  @override
  void initState() {
    super.initState();
    _source = DatabaseBoardSource(a);
  }

  @override
  void didUpdateWidget(BoardView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.args.store != a.store) {
      _source.dispose();
      _source = DatabaseBoardSource(a);
    } else {
      _source.args = a;
    }
  }

  ViewArgs get a => widget.args;

  @override
  void dispose() {
    _source.dispose();
    super.dispose();
  }

  double get _colWidth {
    final m = context.mobile;
    return switch (a.view.cfg.cardSize) {
      CardSize.small => m ? 220.0 : 220.0,
      CardSize.medium => m ? 276.0 : Db.boardColumn,
      CardSize.large => m ? 300.0 : 340.0,
    };
  }

  @override
  Widget build(BuildContext context) {
    if (a.store.groupDef == null) return _noGroup(context);
    _source.colors = context.colors;
    final hidden = [
      for (final g in a.store.groups)
        if (a.view.cfg.hiddenGroups.contains(g.key)) g,
    ];
    return ListenableBuilder(
      listenable: noteTypeRegistry,
      builder: (context, _) => BoardRenderer(
        source: _source,
        inline: a.inline,
        gutter: a.gutter,
        columnWidth: _colWidth,
        columnGap: Db.boardGap,
        inlinePageSize: inlinePage,
        cardBuilder: _card,
        columnLabelBuilder: _columnLabel,
        columnMenu: _groupMenu,
        trailing: hidden.isEmpty
            ? null
            : _HiddenColumn(state: this, groups: hidden),
      ),
    );
  }

  Widget _card(
    BuildContext context,
    BoardColumn column,
    BoardCard card,
    bool lifted,
  ) {
    final row = _source.row(column.id, card.id);
    final noteArgs = NoteArgs(
      pageId: row.id,
      pageType: row.pageType,
      title: row.title,
      icon: row.icon,
      onOpen: () => _source.openCard(context, card),
    );
    final boardCard = noteTypeRegistry.forPage(noteArgs)?.boardCard;
    if (boardCard != null) return boardCard(context, noteArgs);
    final cfg = a.view.cfg;
    final g = _source.group(column.id);
    final colored =
        cfg.colorColumns &&
        g.key.isNotEmpty &&
        a.store.groupDef!.kind != PropKind.checkbox;
    final tint = colored && context.colors.dark ? g.group.color : null;
    final coverH = cfg.preview == 'none'
        ? null
        : (cfg.cardSize == CardSize.large ? 200.0 : 150.0);
    if (lifted) {
      return RowCard(
        args: a,
        row: row,
        tint: tint,
        coverHeight: coverH,
        contentPreview: cfg.preview == 'content',
        lifted: true,
      );
    }
    return Builder(
      builder: (cell) {
        editTitleWhenBuilt(cell, a.ctl, row);
        return RowCard(
          key: ValueKey('rc-${row.id}'),
          args: a,
          row: row,
          tint: tint,
          coverHeight: coverH,
          contentPreview: cfg.preview == 'content',
        );
      },
    );
  }

  Widget _columnLabel(BuildContext context, BoardColumn column) {
    final g = _source.group(column.id);
    final gd = a.store.groupDef!;
    final label = _source.label(g);
    final c = context.colors;
    if (label == null) {
      return Text(
        'No ${gd.name}',
        style: ts(
          context.mobile ? const TypeSpec(15, 20, 500) : context.type.ui,
          c.textPrimary,
          weight: 500,
        ),
      );
    }
    if (gd.kind == PropKind.checkbox) {
      return Text(
        label,
        style: ts(context.type.ui, c.textPrimary, weight: 500),
      );
    }
    return Flexible(
      child: Pill(
        label,
        color: g.group.color,
        status: gd.kind == PropKind.status,
        group: gd.kind == PropKind.status ? statusGroupOf(gd, g.key) : null,
      ),
    );
  }

  void _groupMenu(BuildContext anchor, BoardColumn column) => showPanel<void>(
    anchor,
    globalRectOf(anchor),
    (ctx) => Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        dbItem(
          ctx,
          'Hide group',
          DbIco.eyeOff,
          key: const ValueKey('group-hide'),
          onTap: () {
            Navigator.of(ctx).pop();
            a.store.updateConfig(
              a.view,
              (c) => c.hiddenGroups = {...c.hiddenGroups, column.id},
            );
          },
        ),
      ],
    ),
    width: 220,
  );

  Widget _noGroup(BuildContext context) {
    final c = context.colors;
    final cands = [
      for (final d in a.store.schema)
        if (propertyRegistry.of(d.kind)?.groupable ?? false) d,
    ];
    return Padding(
      padding: EdgeInsets.fromLTRB(a.gutter, 24, a.gutter, 24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            'Group the board by a property',
            style: ts(context.type.ui, c.textSecondary),
          ),
          const SizedBox(height: 8),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              for (final d in cands)
                ChipBtn(
                  key: ValueKey('board-group-${d.name}'),
                  label: d.name,
                  icon: propIcon(d),
                  onTap: () =>
                      a.store.updateView(a.view.copyWith(groupBy: d.key)),
                ),
              ChipBtn(
                key: const ValueKey('board-group-new'),
                label: 'New status property',
                icon: DbIco.status,
                onTap: () async {
                  final k = await a.store.addProperty(
                    PropKind.status,
                    name: 'Status',
                    inView: a.view,
                  );
                  final v = a.store.view;
                  if (v != null) {
                    await a.store.updateView(v.copyWith(groupBy: k));
                  }
                },
              ),
            ],
          ),
        ],
      ),
    );
  }
}

/// "Hidden groups": the names and counts of hidden groups; a tap shows one
/// again.
class _HiddenColumn extends StatelessWidget {
  const _HiddenColumn({required this.state, required this.groups});
  final _BoardViewState state;
  final List<GroupSource> groups;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final a = state.a;
    final gd = a.store.groupDef!;
    return SizedBox(
      width: 220,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          SizedBox(
            height: 34,
            child: Align(
              alignment: Alignment.centerLeft,
              child: Text(
                'Hidden groups',
                style: ts(context.type.ui, c.textTertiary, weight: 500),
              ),
            ),
          ),
          for (final g in groups)
            Hover(
              key: ValueKey('hidden-${g.key}'),
              onTap: () => a.store.updateConfig(
                a.view,
                (cfg) =>
                    cfg.hiddenGroups = {...cfg.hiddenGroups}..remove(g.key),
              ),
              radius: Radii.md,
              padding: const EdgeInsets.symmetric(horizontal: 6),
              child: SizedBox(
                height: 32,
                child: Row(
                  children: [
                    if (g.key.isEmpty)
                      Text(
                        'No ${gd.name}',
                        style: ts(context.type.ui, c.textSecondary),
                      )
                    else
                      Flexible(
                        child: Pill(
                          g.group.label,
                          color: g.group.color,
                          status: gd.kind == PropKind.status,
                          group: gd.kind == PropKind.status
                              ? statusGroupOf(gd, g.key)
                              : null,
                        ),
                      ),
                    const SizedBox(width: 6),
                    Text(
                      '${g.group.count}',
                      style: ts(context.type.ui, c.textTertiary),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}
