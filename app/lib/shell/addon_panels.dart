import 'package:flutter/widgets.dart';

import 'addons.dart';

class AddonPanelShell extends StatelessWidget {
  const AddonPanelShell({super.key, required this.child});
  final Widget child;
  @override
  Widget build(BuildContext context) {
    var result = child;
    for (final panel in appAddonContext.shellPanels.all) {
      result = panel.wrap(context, result);
    }
    return result;
  }
}

class AddonHeaderActions extends StatelessWidget {
  const AddonHeaderActions({super.key});
  @override
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      for (final panel in appAddonContext.shellPanels.all)
        panel.headerAction(context),
    ],
  );
}
