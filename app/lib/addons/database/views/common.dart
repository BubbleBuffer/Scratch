// Pieces shared by the database views: a row's title with its icon, a
// value through the property registry, "+ New", "Load more", and the
// limit of rows an inline database shows before "Load more".

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../registry/view_registry.dart';
import '../../../state/app_state.dart';
import '../../../state/pages_store.dart';
import '../../../ui/database/values.dart';
import '../../../registry/property_registry.dart';
import '../../../ui/ui.dart';
import '../../../ui/database/db_tokens.dart';

/// Rows an inline database shows at first, and per "Load more".
const inlinePage = 25;

/// The value of [def] in [row] drawn for [mode] (nothing when unknown).
Widget valueOf(BuildContext context, PropDef def, PageRow row, CellMode mode, {bool wrap = false}) {
  final t = propertyRegistry.of(def.kind);
  if (t == null) return const SizedBox.shrink();
  return t.cell(CellArgs(context: context, def: def, value: row.valueOf(def), mode: mode, wrap: wrap));
}

/// The first line of [row]'s body when the row is untitled (its stored
/// meta, see [PagesStore.meta]); empty for a titled row, so no page loads
/// for it. [listen]: rebuild when the meta arrives.
String untitledLine(BuildContext context, PageRow row, {bool listen = true}) {
  if (row.title.trim().isNotEmpty) return '';
  final pages = listen ? context.watch<PagesStore>() : context.read<PagesStore>();
  return pages.meta(row.id)?.firstLine ?? '';
}

/// A row's icon and title.
class RowTitle extends StatelessWidget {
  const RowTitle({
    super.key,
    required this.row,
    required this.style,
    this.iconSize,
    this.showIcon = true,
    this.wrap = false,
  });
  final PageRow row;
  final TextStyle style;
  /// The page icon's box; [IconSize.md] by default.
  final double? iconSize;
  final bool showIcon;
  final bool wrap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final empty = row.title.isEmpty;
    final iconSize = this.iconSize ?? IconSize.md.px;
    return Row(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
      if (showIcon) ...[
        Padding(
          padding: EdgeInsets.only(top: (lineExtent(style) - iconSize) / 2),
          child: PageIcon(icon: row.icon, size: iconSize, color: c.iconSecondary),
        ),
        const Gap(Space.xs),
      ],
      Flexible(
        child: Text(
          displayTitle(row.title, firstLine: untitledLine(context, row)),
          maxLines: wrap ? null : 1,
          softWrap: wrap,
          overflow: wrap ? null : TextOverflow.ellipsis,
          style: empty ? style.copyWith(color: c.textTertiary) : style,
        ),
      ),
    ]);
  }
}

/// Opens the title editor of a row added from the view as soon as its
/// cell is built (the row the controller marked).
void editTitleWhenBuilt(BuildContext cell, ViewController ctl, PageRow row) {
  if (ctl.editTitleOf != row.id) return;
  ctl.editTitleOf = null;
  SchedulerBinding.instance.addPostFrameCallback((_) {
    if (cell.mounted) ctl.editTitle(cell, row);
  });
}

/// The grey "+ New" (or "+ New page") row of views.
class AddRowButton extends StatelessWidget {
  const AddRowButton({super.key, required this.label, required this.onTap, this.size = ControlSize.md, this.padding = Space.sm});
  final String label;
  final VoidCallback onTap;

  /// The row's least height; it grows with the text.
  final ControlSize size;
  final double padding;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Hover(
      onTap: onTap,
      radius: Radii.sm,
      padding: EdgeInsets.symmetric(horizontal: padding),
      child: ConstrainedBox(
        constraints: BoxConstraints(minHeight: size.height),
        child: Row(children: [
          Icon20(Ico.plus, size: IconSize.xs, color: c.iconTertiary),
          const Gap(Space.xs),
          Text(label, style: ts(context.type.ui, c.textTertiary, weight: Weight.regular)),
        ]),
      ),
    );
  }
}

/// "Load more" under an inline database's rows.
class LoadMore extends StatelessWidget {
  const LoadMore({super.key, required this.onTap});
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Hover(
      key: const ValueKey('load-more'),
      onTap: onTap,
      radius: Radii.sm,
      padding: const EdgeInsets.symmetric(horizontal: Space.sm),
      child: ConstrainedBox(
        constraints: BoxConstraints(minHeight: ControlSize.md.height),
        child: Row(children: [
          Icon20(Ico.arrowDown, size: IconSize.xs, color: c.iconTertiary),
          const Gap(Space.xs),
          Text('Load more', style: ts(context.type.ui, c.textTertiary, weight: Weight.regular)),
        ]),
      ),
    );
  }
}

/// The "OPEN" chip on a row's title (on hover on desktop, always on
/// mobile).
class OpenChip extends StatelessWidget {
  const OpenChip({super.key, required this.onTap, this.label = 'OPEN'});
  final VoidCallback onTap;
  final String label;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.touch;
    return Hover(
      onTap: onTap,
      radius: Radii.sm,
      hoverColor: c.bgHover,
      child: Container(
        constraints: BoxConstraints(minHeight: m ? DbOpenChip.heightTouch : DbOpenChip.height),
        padding: EdgeInsets.symmetric(horizontal: Space.xs),
        decoration: BoxDecoration(
          color: c.bgPrimary,
          borderRadius: BorderRadius.circular(Radii.sm),
          boxShadow: c.shadowSmall,
          border: Border.all(color: c.borderTranslucent),
        ),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          if (!m) ...[
            Container(
              width: DbOpenChip.glyph.width,
              height: DbOpenChip.glyph.height,
              decoration: BoxDecoration(border: Border.all(color: c.iconTertiary, width: Stroke.hair), borderRadius: BorderRadius.circular(Radii.sm)),
              alignment: Alignment.centerRight,
              child: Container(width: DbOpenChip.glyphPane, color: c.iconTertiary),
            ),
            const Gap(Space.xxs),
          ],
          Text(label, style: eyebrowStyle(context.type.caption, c.textTertiary, weight: Weight.strong)),
        ]),
      ),
    );
  }
}
