// Link to page and sub-page (both the core's `linktopage`, target = page
// id): the page's icon and its title, 16/20.8 500 with a thin underline. A
// link to a page elsewhere shows a small arrow on its icon; a sub-page (a
// child of this page) does not. `/page` creates a sub-page; `/link` picks
// an existing page.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../state/app_state.dart';
import '../state/pages_store.dart';
import '../ui/block_colors.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import 'media.dart';

class PageLinkRow extends StatelessWidget {
  const PageLinkRow({super.key, required this.pageId, required this.fromPage, this.onTap});
  final String pageId;

  /// The page the link is on (a child of it is a sub-page).
  final String? fromPage;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final meta = context.watch<PagesStore>().meta(pageId);
    final sub = meta != null && meta.parent == fromPage;
    final gone = meta != null && (meta.missing || meta.inTrash);
    final title = meta == null ? '' : (gone ? (meta.missing ? 'Page not found' : '${displayTitle(meta.title, firstLine: meta.firstLine)} (in trash)') : displayTitle(meta.title, firstLine: meta.firstLine));
    final icon = meta?.icon;
    return Hover(
      radius: Radii.sm,
      onTap: onTap,
      child: SizedBox(
        height: 28,
        child: Row(children: [
          SizedBox(
            width: 24,
            height: 24,
            child: Stack(clipBehavior: Clip.none, children: [
              Center(
                child: icon != null && icon.isNotEmpty
                    ? Text(icon, style: TextStyle(fontSize: 17, height: 1, fontFamilyFallback: emojiFallback))
                    : Icon20(Ico.page, size: 20, color: c.iconSecondary),
              ),
              if (!sub)
                Positioned(right: -1, bottom: 0, child: IconX(Ix.linkPage, size: 10, color: c.iconPrimary, stroke: 2)),
            ]),
          ),
          const SizedBox(width: 6),
          Flexible(
            child: Container(
              decoration: BoxDecoration(border: Border(bottom: BorderSide(color: c.divider))),
              child: Text(title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: tsRaw(16, 20.8, 500, gone ? c.textTertiary : c.textPrimary)),
            ),
          ),
        ]),
      ),
    );
  }
}

Widget _body(BlockContext b) {
  final n = b.node;
  final t = n.target;
  if (t == null || t.isEmpty) {
    return emptyMedia(b, Ix.linkPage, 'Link to a page');
  }
  return PageLinkRow(
    key: ValueKey('pagelink-${n.id}'),
    pageId: t,
    fromPage: b.host.pageId,
    onTap: () => b.host.openPage(t),
  );
}

const linkToPageBlock = BlockType(
  kind: Kinds.linkToPage,
  label: 'Link to page',
  hint: 'Link to an existing page.',
  iconX: Ix.linkPage,
  aliases: 'link page mention',
  editable: false,
  acceptsChildren: false,
  colorable: false,
  insertOnly: false,
  body: _body,
);

/// `/page`: creates a sub-page and links it here.
const subPageEntry = BlockType(
  kind: 'subpage',
  label: 'Page',
  hint: 'Embed a sub-page inside this page.',
  icon: Ico.page,
  aliases: 'page subpage new',
  editable: false,
  insertOnly: true,
);
