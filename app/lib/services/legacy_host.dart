// Transitional host for add-ons. Navigation, page history, templates and the
// block pickers are not add-ons yet, so add-ons reach them only through this
// file, and only the names listed here. Remove an export when its feature
// becomes an add-on. The bridge is temporary; see wiki/app/addons.md.

export '../shell/navigation.dart' show openPage;
export '../shell/search.dart' show showSearchOverlay;
export '../shell/mobile_page_history.dart' show MobilePageEntryScope;
export '../features/templates/templates.dart'
    show showTemplateMenu, withTemplates;
export '../editor/block_pickers.dart' show pickAttachment, pickPage;
export '../shell/commands.dart' show registerShellCommands;
