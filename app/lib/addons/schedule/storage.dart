// Where a schedule's definition and status live: the page's type data. The
// view and controller see only [ScheduleStorage], so tests can use an
// in-memory copy (fake.dart) and the app uses the vault.

import 'package:flutter/widgets.dart';

import '../../core/vault.dart';

abstract class ScheduleStorage {
  /// The JSON object of one add-on area on [pageId] ('{}' when absent).
  Future<String> area(String pageId, String addon);

  /// Replaces one top-level key of [addon]'s area with [jsonValue].
  Future<void> setKey(
    String pageId,
    String addon,
    String key,
    String jsonValue,
  );

  /// Removes one top-level key of [addon]'s area.
  Future<void> removeKey(String pageId, String addon, String key);

  /// The title of [pageId], or null when there is no such page.
  Future<String?> titleOf(String pageId);
}

/// Storage on the vault. Writes persist, sync and wake the change feed.
class VaultScheduleStorage implements ScheduleStorage {
  const VaultScheduleStorage(this.vault);

  final Vault vault;

  @override
  Future<String> area(String pageId, String addon) =>
      vault.typeData(pageId, addon);

  @override
  Future<void> setKey(
    String pageId,
    String addon,
    String key,
    String jsonValue,
  ) => vault.typeDataSet(pageId, addon, [key], jsonValue);

  @override
  Future<void> removeKey(String pageId, String addon, String key) =>
      vault.typeDataDelete(pageId, addon, [key]);

  @override
  Future<String?> titleOf(String pageId) async {
    try {
      return (await vault.loadPage(pageId)).title;
    } catch (_) {
      return null;
    }
  }
}

/// The storage the schedule widgets use, from the nearest scope.
class ScheduleScope extends InheritedWidget {
  const ScheduleScope({
    super.key,
    required this.storage,
    required super.child,
  });

  final ScheduleStorage storage;

  static ScheduleStorage storageOf(BuildContext context) {
    final scope = context.dependOnInheritedWidgetOfExactType<ScheduleScope>();
    assert(scope != null, 'ScheduleScope is missing above the schedule view');
    return scope!.storage;
  }

  @override
  bool updateShouldNotify(ScheduleScope old) => old.storage != storage;
}
