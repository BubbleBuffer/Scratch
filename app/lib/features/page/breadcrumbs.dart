// The desktop top bar's breadcrumbs: the page's ancestors and the page,
// each with its icon, separated by "/". More than three ancestors collapse
// the middle ones into "…".

import 'package:flutter/widgets.dart';

import '../../shell/navigation.dart';
import '../../ui/database/db_icons.dart';
import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/ui.dart';

class Breadcrumbs extends StatelessWidget {
  const Breadcrumbs({
    super.key,
    required this.pageId,
    required this.title,
    this.icon,
  });
  final String pageId;

  /// The current page's title and icon (live from the editor).
  final String title;
  final String? icon;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final pages = context.watch<PagesStore>();
    var crumbs = pages.ancestors(pageId);
    final collapsed = crumbs.length > 3;
    if (collapsed) {
      crumbs = [crumbs.first, ...crumbs.sublist(crumbs.length - 2)];
    }

    SpacePatch? patchFor(String key) {
      final id = key == 'crumb-current' ? pageId : key.substring(6);
      final ancestors = pages.ancestors(id);
      return spacePatch(
        ancestors.isEmpty ? pages.meta(id)?.color : ancestors.first.color,
        c.dark ? Brightness.dark : Brightness.light,
      );
    }

    Widget crumb(
      String key,
      String? icon,
      String title,
      VoidCallback? onTap, {
      double maxWidth = 160,
      bool database = false,
    }) {
      final patch = patchFor(key);
      final id = key == 'crumb-current' ? pageId : key.substring(6);
      final label = pageLabelOf(context, id, title);
      final style = ts(ty.ui, c.textPrimary, weight: onTap == null ? 700 : 400);
      final glyph = database && (icon == null || icon.isEmpty)
          ? patch == null
                ? DbIcon(DbIco.database, size: 16, color: c.iconSecondary)
                : Container(
                    width: 22,
                    height: 22,
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      color: patch.fill,
                      borderRadius: Radii.ctl,
                    ),
                    child: DbIcon(DbIco.database, size: 16, color: patch.ink),
                  )
          : SpaceIconTile(
              icon: icon,
              size: 22,
              glyphSize: 16,
              color: c.iconSecondary,
              patch: patch,
            );
      return Hover(
        key: ValueKey(key),
        radius: Radii.sm,
        onTap: onTap,
        padding: const EdgeInsets.symmetric(horizontal: 6),
        child: SizedBox(
          height: 24,
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              glyph,
              const SizedBox(width: 6),
              ConstrainedBox(
                constraints: BoxConstraints(maxWidth: maxWidth),
                child: Text(
                  label.text,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: label.firstLine
                      ? firstLineStyle(style, c.textTertiary)
                      : style,
                ),
              ),
            ],
          ),
        ),
      );
    }

    Widget slash() => Padding(
      padding: const EdgeInsets.symmetric(horizontal: 1),
      child: Text('/', style: ts(ty.ui, c.textTertiary, weight: 400)),
    );

    final out = <Widget>[];
    for (var i = 0; i < crumbs.length; i++) {
      final m = crumbs[i];
      out.add(
        crumb(
          'crumb-${m.id}',
          m.icon,
          m.title,
          () => openPage(context, m.id, m.title, stack: true),
          database: m.isDatabase,
        ),
      );
      out.add(slash());
      if (collapsed && i == 0) {
        out.add(
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 6),
            child: Text('…', style: ts(ty.ui, c.textTertiary, weight: 400)),
          ),
        );
        out.add(slash());
      }
    }
    out.add(crumb('crumb-current', icon, title, null, maxWidth: 280));
    // A narrow window clips the trail rather than overflowing the bar.
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      physics: const NeverScrollableScrollPhysics(),
      child: Row(mainAxisSize: MainAxisSize.min, children: out),
    );
  }
}
