// Web bookmark: a card for the URL in the block's target (radius 10, about
// 108 tall): the page's title, description and favicon once
// fetched (see LinkPreviews), else the site name; then the URL. Tapping it
// selects the block.

import 'package:flutter/widgets.dart';

import '../core/link_previews.dart';
import '../core/models.dart';
import '../registry/block_registry.dart';
import '../state/app_state.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import 'media.dart';

/// "example.com" for "https://www.example.com/a/b".
String siteName(String url) {
  final u = Uri.tryParse(url.contains('://') ? url : 'https://$url');
  final h = u?.host ?? '';
  if (h.isEmpty) return url;
  return h.startsWith('www.') ? h.substring(4) : h;
}

Widget _decorate(BlockContext b, Widget text) {
  final t = b.node.target;
  final content = t == null || t.isEmpty
      ? emptyMedia(b, Ix.bookmark, 'Add a web bookmark')
      : _Card(key: ValueKey('bookmark-${b.node.id}'), url: t);
  return withCaption(b, content, text);
}

class _Card extends StatelessWidget {
  const _Card({super.key, required this.url});
  final String url;

  @override
  Widget build(BuildContext context) {
    final links = context.app.links;
    return ListenableBuilder(listenable: links, builder: (context, _) => _card(context, links.of(url)));
  }

  Widget _card(BuildContext context, LinkPreview? p) {
    final c = context.colors;
    final title = p?.title;
    final desc = p?.description;
    final icon = p?.icon;
    final fallback = IconX(Ix.bookmark, size: 14, color: c.iconSecondary);
    return Hover(
      radius: Radii.lg,
      child: Container(
        constraints: const BoxConstraints(minHeight: 72),
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 14),
        decoration: BoxDecoration(
          border: Border.all(color: c.border),
          borderRadius: BorderRadius.circular(Radii.lg),
        ),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
          Text(title ?? siteName(url),
              key: ValueKey('bookmark-title-$url'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: tsRaw(14, 20, 400, c.textPrimary)),
          if (desc != null) ...[
            const SizedBox(height: 4),
            Text(desc,
                key: ValueKey('bookmark-description-$url'),
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: tsRaw(12, 16, 400, c.textSecondary)),
          ],
          const SizedBox(height: 8),
          Row(children: [
            if (icon != null)
              SizedBox(
                key: ValueKey('bookmark-favicon-$url'),
                width: 14,
                height: 14,
                child: Image.memory(icon,
                    width: 14, height: 14, fit: BoxFit.contain, errorBuilder: (_, _, _) => fallback),
              )
            else
              fallback,
            const SizedBox(width: 6),
            Flexible(
              child: Text(url, maxLines: 1, overflow: TextOverflow.ellipsis, style: tsRaw(12, 16, 400, c.textPrimary)),
            ),
          ]),
        ]),
      ),
    );
  }
}

const bookmarkBlock = BlockType(
  kind: Kinds.bookmark,
  label: 'Web bookmark',
  hint: 'Save a link as a visual bookmark.',
  iconX: Ix.bookmark,
  aliases: 'bookmark link url web',
  placeholder: 'Add a caption…',
  textStyle: captionStyle,
  decorate: _decorate,
  handleTop: _top,
  atomic: true,
  acceptsChildren: false,
  colorable: false,
  group: 'Media',
);

double _top(BlockContext b) => (b.mobile ? 3 : 2) + 12;
