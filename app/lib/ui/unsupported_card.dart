import 'package:flutter/widgets.dart';

import 'app_name.dart';
import 'tokens.dart';

/// The card for a block kind this version does not know. It is read-only and
/// does nothing on tap, so it has no hover or lift. The first line says the
/// block is not supported; the muted line names its kind and text.
class UnsupportedCard extends StatelessWidget {
  const UnsupportedCard({super.key, required this.kind, required this.text});

  final String kind;
  final String text;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final type = context.type;
    final message = 'Not supported on this version of $appName';
    final detail = text.isEmpty ? kind : '$kind · $text';
    return Semantics(
      label: '$message. $detail',
      excludeSemantics: true,
      child: Container(
        width: double.infinity,
        padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
        decoration: BoxDecoration(
          color: c.bgPrimary,
          border: Border.all(color: c.border),
          borderRadius: Radii.panel,
          boxShadow: c.seam,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.only(top: 1, right: 10),
              child: Image.asset(
                'assets/icons/app_icon_64.png',
                width: 20,
                height: 20,
                filterQuality: FilterQuality.medium,
                excludeFromSemantics: true,
              ),
            ),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(message, style: ts(type.ui, c.textPrimary, weight: 500)),
                  const SizedBox(height: 2),
                  Text(
                    detail,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: ts(type.uiSmall, c.textTertiary),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
