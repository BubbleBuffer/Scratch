// The card of board and gallery views: an optional preview, the title with
// its icon, and the shown properties that have a value.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../ui/database/config.dart';
import '../cells.dart';
import '../../../ui/database/values.dart';
import '../../../registry/property_registry.dart';
import '../../../registry/view_registry.dart';
import '../../../ui/ui.dart';
import 'common.dart';
import 'cover.dart';

class RowCard extends StatefulWidget {
  const RowCard({
    super.key,
    required this.args,
    required this.row,
    this.tint,
    this.coverHeight,
    this.contentPreview = false,
    this.lifted = false,
    this.width,
  });
  final ViewArgs args;
  final PageRow row;

  /// The group colour of a coloured board column (dark mode tints cards).
  final String? tint;

  /// Height of the preview (null: none).
  final double? coverHeight;
  final bool contentPreview;

  /// Drawn while dragged.
  final bool lifted;
  final double? width;

  @override
  State<RowCard> createState() => _RowCardState();
}

class _RowCardState extends State<RowCard> {
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final a = widget.args;
    final r = widget.row;
    final m = context.mobile;
    final group = a.view.groupBy;
    final props = [
      for (final d in a.store.visibleProps(a.view))
        if (d.key != group && shownOnCard(r.valueOf(d))) d,
    ];
    final bg = c.bgPrimary;
    final title = ts(
      m ? const TypeSpec(15, 20, 500) : const TypeSpec(14, 20, 500),
      c.textPrimary,
    );
    return Hover(
      key: ValueKey('card-${r.title}'),
      onTap: () => a.ctl.openRow(context, r.id, title: r.title),
      borderRadius: Radii.panel,
      lift: 2,
      background: bg,
      hoverBackground: Color.alphaBlend(c.bgHover, bg),
      border: Border.all(color: c.border),
      shadow: widget.lifted ? c.seamHover : c.seam,
      hoverShadow: c.seamHover,
      child: SizedBox(
        width: widget.width,
        child: ClipRRect(
          borderRadius: Radii.panel,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (widget.coverHeight != null)
                CardCover(
                  vault: a.store.vault,
                  row: r,
                  height: widget.coverHeight!,
                  content: widget.contentPreview,
                ),
              Padding(
                padding: EdgeInsets.fromLTRB(10, m ? 10 : 8, 10, m ? 10 : 8),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    RowTitle(
                      row: r,
                      style: title,
                      iconSize: m ? 18 : 16,
                      showIcon: a.view.cfg.showIcon,
                      wrap: true,
                    ),
                    for (final d in props)
                      Padding(
                        padding: const EdgeInsets.only(top: 6),
                        child: valueOf(
                          context,
                          d,
                          r,
                          CellMode.card,
                          wrap: true,
                        ),
                      ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
