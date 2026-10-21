import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../services/addon_services.dart';
import '../../ui/ui.dart';
import 'task_composer.dart';

Future<void> openTask(BuildContext context, String page) async {
  final data = await const AddonServices().vault(context).loadPage(page);
  if (context.mounted) await openPage(context, page, data.title, stack: true);
}

/// Completes with the new page id after its fields have persisted.
Future<String?> showTaskComposer(
  BuildContext context, {
  DateTime? date,
  String? calendar,
  int? hour,
  bool fromToday = false,
}) async {
  var openFull = false;
  Widget content(BuildContext routeContext) => TaskComposer(
    initialDate: date ?? clockNow(),
    initialCalendar: calendar,
    initialHour: hour,
    fromToday: fromToday,
    onCreated: (id) => Navigator.of(routeContext).pop(id),
    onOpenFull: (id) {
      openFull = true;
      Navigator.of(routeContext).pop(id);
    },
  );
  final String? id;
  if (context.mobile) {
    id = await showSheet<String>(
      context,
      (routeContext) => Padding(
        padding: EdgeInsets.only(
          bottom: MediaQuery.viewInsetsOf(routeContext).bottom,
        ),
        child: SheetSurface(children: [content(routeContext)]),
      ),
    );
  } else {
    id = await showModalPanel<String>(
      context,
      (routeContext) => SingleChildScrollView(child: content(routeContext)),
      width: 460,
      height: 680,
      shrink: true,
    );
  }
  if (id != null && openFull && context.mounted) {
    unawaited(openTask(context, id));
  }
  return id;
}
