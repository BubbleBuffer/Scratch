// The gallery view: a grid of cards with a preview (the page cover or its
// first image, or its content), sized small, medium or large. Rows of
// cards are built as they scroll into view.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../../../../core/models.dart';
import '../../../../ui/database/config.dart';
import '../../../../ui/database/db_icons.dart';
import '../../../../ui/database/widgets.dart';
import '../../../../registry/view_registry.dart';
import '../../../../ui/ui.dart';
import '../card.dart';
import '../common.dart';

String _previewLabel(String p) => switch (p) { 'cover' => 'Page cover', 'content' => 'Page content', _ => 'None' };
String _sizeLabel(CardSize s) => '${s.name[0].toUpperCase()}${s.name.substring(1)}';

final galleryView = ViewType(
  kind: ViewKind.gallery,
  label: 'Gallery',
  icon: DbIco.gallery,
  build: (context, a) => GalleryView(args: a),
  options: (context, a) {
    final cfg = a.view.cfg;
    return [
      ViewOption(
        label: 'Card preview',
        value: menuValue(context, _previewLabel(cfg.previewFor(ViewKind.gallery))),
        onTap: (ctx, rect) async {
          final p = await showPanel<String>(
            ctx,
            rect,
            (c) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              for (final id in const ['none', 'cover', 'content'])
                MenuItem(
                  key: ValueKey('preview-$id'),
                  label: _previewLabel(id),
                  trailing: cfg.previewFor(ViewKind.gallery) == id ? Icon20(Ico.check, size: 16, color: c.colors.iconPrimary) : null,
                  onTap: () => Navigator.of(c).pop(id),
                ),
            ]),
            width: 220,
          );
          if (p != null) await a.store.updateConfig(a.view, (c) => c.preview = p);
        },
      ),
      ViewOption(
        label: 'Card size',
        value: menuValue(context, _sizeLabel(cfg.cardSize)),
        onTap: (ctx, rect) async {
          final s = await showPanel<CardSize>(
            ctx,
            rect,
            (c) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              for (final v in CardSize.values)
                MenuItem(
                  key: ValueKey('size-${v.name}'),
                  label: _sizeLabel(v),
                  trailing: cfg.cardSize == v ? Icon20(Ico.check, size: 16, color: c.colors.iconPrimary) : null,
                  onTap: () => Navigator.of(c).pop(v),
                ),
            ]),
            width: 220,
          );
          if (s != null) await a.store.updateConfig(a.view, (c) => c.cardSize = s);
        },
      ),
      ViewOption(
        label: 'Show page icon',
        value: Toggle(on: cfg.showIcon),
        onTap: (_, _) => a.store.updateConfig(a.view, (c) => c.showIcon = !c.showIcon),
      ),
    ];
  },
);

class GalleryView extends StatefulWidget {
  const GalleryView({super.key, required this.args});
  final ViewArgs args;

  @override
  State<GalleryView> createState() => _GalleryViewState();
}

class _GalleryViewState extends State<GalleryView> {
  int _limit = inlinePage;
  final _v = ScrollController();

  ViewArgs get a => widget.args;

  @override
  void dispose() {
    _v.dispose();
    super.dispose();
  }

  Future<void> _add() async {
    await a.ctl.newRow(context);
  }

  @override
  Widget build(BuildContext context) {
    final cfg = a.view.cfg;
    final m = context.mobile;
    final minW = switch (cfg.cardSize) {
      CardSize.small => m ? 150.0 : 180.0,
      CardSize.medium => m ? 300.0 : 260.0,
      CardSize.large => m ? 320.0 : 340.0,
    };
    final preview = cfg.previewFor(ViewKind.gallery);
    final coverH = preview == 'none'
        ? null
        : switch (cfg.cardSize) { CardSize.small => 110.0, CardSize.medium => 150.0, CardSize.large => 220.0 };
    const gap = 16.0;
    final src = a.store.main;
    final total = math.max(0, src.total);
    final n = a.inline ? math.min(total, _limit) : total;

    return LayoutBuilder(builder: (context, box) {
      final avail = (a.inline ? (box.maxWidth - a.gutter) : box.maxWidth - 2 * a.gutter).clamp(minW, double.infinity);
      final perRow = math.max(1, ((avail + gap) / (minW + gap)).floor());
      final cardW = (avail - gap * (perRow - 1)) / perRow;
      final rows = (n / perRow).ceil();

      Widget gridRow(int ri) => Padding(
            padding: const EdgeInsets.only(bottom: gap),
            child: IntrinsicHeight(
              child: Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                for (var k = 0; k < perRow; k++) ...[
                  if (k > 0) const SizedBox(width: gap),
                  SizedBox(width: cardW, child: _card(ri * perRow + k, n, coverH, preview)),
                ],
              ]),
            ),
          );

      final add = SizedBox(
        width: cardW,
        child: AddRowButton(key: const ValueKey('gallery-new'), label: 'New page', onTap: _add, height: 34),
      );

      if (a.inline) {
        return Padding(
          padding: EdgeInsets.only(left: a.gutter, top: 4),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
            for (var i = 0; i < rows; i++) gridRow(i),
            if (total > n) LoadMore(onTap: () => setState(() => _limit += inlinePage)),
            add,
          ]),
        );
      }
      return ListView.builder(
        key: const ValueKey('gallery-rows'),
        controller: _v,
        padding: EdgeInsets.fromLTRB(a.gutter, 4, a.gutter, 120),
        itemCount: rows + 1,
        itemBuilder: (context, i) => i < rows ? gridRow(i) : Align(alignment: Alignment.centerLeft, child: add),
      );
    });
  }

  Widget _card(int i, int n, double? coverH, String preview) {
    if (i >= n) return const SizedBox.shrink();
    final r = a.store.main.rowAt(i);
    if (r == null) return SizedBox(height: (coverH ?? 0) + 60);
    return Builder(builder: (cell) {
      editTitleWhenBuilt(cell, a.ctl, r);
      return RowCard(key: ValueKey('gc-${r.id}'), args: a, row: r, coverHeight: coverH, contentPreview: preview == 'content');
    });
  }
}
