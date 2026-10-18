//! The menu bar UI: one status item, one menu, one controller object.
//!
//! Memory discipline: every AppKit object is held through `Retained<_>` and
//! released by Rust's drop rules. The controller is the target of the menu
//! items (AppKit holds targets weakly) and of the repeating timer (which
//! retains its target), so the timer is always invalidated before it is
//! dropped, breaking the only cycle this design could form.

use std::cell::RefCell;
use std::time::Duration;

use bluemeth_core::{AssertionKind, Inhibitor, format_duration, parse_duration};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSApplicationActivationPolicy,
    NSApplicationDelegate, NSCellImagePosition, NSControlStateValue, NSControlStateValueMixed,
    NSControlStateValueOff, NSControlStateValueOn, NSFont, NSFontWeightRegular, NSImage, NSMenu,
    NSMenuDelegate, NSMenuItem, NSStatusBar, NSStatusItem, NSTextField, NSVariableStatusItemLength,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes,
    NSSize, NSString, NSTimer, ns_string,
};

use crate::format::countdown;
use crate::login_item::{self, LoginItem};
use crate::prefs;
use crate::presets::{PRESETS, is_preset};
use crate::settings::{MENU_KINDS, Settings};

const APP_NAME: &str = "Bluemeth";
const SYMBOL_ACTIVE: &str = "cup.and.saucer.fill";
const SYMBOL_IDLE: &str = "cup.and.saucer";
/// Tag of the "Custom…" timeout item; presets use their seconds as tag.
const CUSTOM_TAG: isize = -1;

struct State {
    settings: Settings,
    inhibitor: Inhibitor,
    last_error: Option<String>,
}

struct Ui {
    status_item: Retained<NSStatusItem>,
    image_active: Option<Retained<NSImage>>,
    image_idle: Option<Retained<NSImage>>,
    menu: Retained<NSMenu>,
    header: Retained<NSMenuItem>,
    error_item: Retained<NSMenuItem>,
    kind_items: Vec<(AssertionKind, Retained<NSMenuItem>)>,
    user_active_item: Retained<NSMenuItem>,
    timeout_items: Vec<Retained<NSMenuItem>>,
    custom_item: Retained<NSMenuItem>,
    enable_all_item: Retained<NSMenuItem>,
    stop_all_item: Retained<NSMenuItem>,
    login_item: Retained<NSMenuItem>,
    quit_item: Retained<NSMenuItem>,
}

struct Ivars {
    state: RefCell<State>,
    ui: Ui,
    timer: RefCell<Option<Retained<NSTimer>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Controller` does
    // not implement `Drop` (its ivars drop on dealloc).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "BluemethController"]
    #[ivars = Ivars]
    struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSApplicationDelegate for Controller {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            self.apply_settings();
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn application_will_terminate(&self, _notification: &NSNotification) {
            self.stop_timer();
            let mut state = self.ivars().state.borrow_mut();
            if let Err(err) = state.inhibitor.release_all() {
                eprintln!("{APP_NAME}: {err}");
            }
        }
    }

    unsafe impl NSMenuDelegate for Controller {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            self.sync_ui();
        }
    }

    impl Controller {
        #[unsafe(method(toggleKind:))]
        fn toggle_kind(&self, sender: &NSMenuItem) {
            let index = usize::try_from(sender.tag()).unwrap_or(usize::MAX);
            let Some(kind) = MENU_KINDS.get(index).copied() else {
                return;
            };
            {
                let mut state = self.ivars().state.borrow_mut();
                // Flip relative to what is *really* held, so a failed
                // assertion shown as "off" turns on when clicked.
                let on_now = state.inhibitor.current().is_enabled(kind);
                state.settings.set_kind(kind, !on_now);
            }
            self.apply_settings();
        }

        #[unsafe(method(toggleUserActive:))]
        fn toggle_user_active(&self, _sender: &NSMenuItem) {
            {
                let mut state = self.ivars().state.borrow_mut();
                let on_now = state.inhibitor.current().declare_user_active;
                state.settings.user_active = !on_now;
            }
            self.apply_settings();
        }

        #[unsafe(method(selectTimeout:))]
        fn select_timeout(&self, sender: &NSMenuItem) {
            let tag = sender.tag();
            let secs = if tag == CUSTOM_TAG {
                match self.prompt_custom_timeout() {
                    Some(secs) => secs,
                    None => return,
                }
            } else {
                u64::try_from(tag).unwrap_or(0)
            };
            self.ivars().state.borrow_mut().settings.timeout_secs = secs;
            self.restart_with_settings();
        }

        #[unsafe(method(enableAll:))]
        fn enable_all(&self, _sender: &NSMenuItem) {
            self.ivars().state.borrow_mut().settings.set_all(true);
            self.apply_settings();
        }

        #[unsafe(method(stopAll:))]
        fn stop_all(&self, _sender: &NSMenuItem) {
            self.ivars().state.borrow_mut().settings.set_all(false);
            self.apply_settings();
        }

        #[unsafe(method(toggleLaunchAtLogin:))]
        fn toggle_launch_at_login(&self, _sender: &NSMenuItem) {
            let enable = !matches!(login_item::status(), LoginItem::Enabled);
            if let Err(err) = login_item::set_enabled(enable) {
                self.show_error("Could not update Launch at Login", &err);
            }
            self.sync_ui();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &NSMenuItem) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            self.on_tick();
        }

        /// Open the menu as if the status item had been clicked.
        #[unsafe(method(openMenu:))]
        fn open_menu(&self, _sender: Option<&AnyObject>) {
            if let Some(button) = self.ivars().ui.status_item.button(self.mtm()) {
                // SAFETY: simulating a click on our own status bar button from
                // the main thread, exactly as a user click would arrive.
                unsafe { button.performClick(None) };
            }
        }
    }
);

impl Controller {
    fn new(mtm: MainThreadMarker, ui: Ui, state: State) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            state: RefCell::new(state),
            ui,
            timer: RefCell::new(None),
        });
        // SAFETY: `init` on an NSObject subclass whose ivars are already set.
        unsafe { msg_send![super(this), init] }
    }

    /// Point every actionable menu item at this controller.
    fn install_targets(&self) {
        let ui = &self.ivars().ui;
        let target: &AnyObject = self;
        let items = ui
            .kind_items
            .iter()
            .map(|(_, item)| item)
            .chain(ui.timeout_items.iter())
            .chain([
                &ui.user_active_item,
                &ui.custom_item,
                &ui.enable_all_item,
                &ui.stop_all_item,
                &ui.login_item,
                &ui.quit_item,
            ]);
        for item in items {
            // SAFETY: NSMenuItem holds its target weakly; the controller is
            // kept alive by `run()` for the whole life of the process, so it
            // always outlives the menu.
            unsafe { item.setTarget(Some(target)) };
        }
        ui.menu.setDelegate(Some(ProtocolObject::from_ref(self)));
    }

    /// Persist settings and reconcile the inhibitor with them.
    fn apply_settings(&self) {
        {
            let mut state = self.ivars().state.borrow_mut();
            prefs::save(&state.settings);
            let config = state.settings.to_config();
            let result = state.inhibitor.apply(&config);
            state.last_error = result.err().map(|e| e.to_string());
        }
        self.after_state_change();
    }

    /// Like `apply_settings`, but restart the countdown from the full timeout.
    fn restart_with_settings(&self) {
        {
            let mut state = self.ivars().state.borrow_mut();
            prefs::save(&state.settings);
            let config = state.settings.to_config();
            let result = if config.any_enabled() {
                state.inhibitor.restart(&config)
            } else {
                state.inhibitor.apply(&config)
            };
            state.last_error = result.err().map(|e| e.to_string());
        }
        self.after_state_change();
    }

    fn after_state_change(&self) {
        if let Some(err) = &self.ivars().state.borrow().last_error {
            eprintln!("{APP_NAME}: {err}");
        }
        self.sync_ui();
        self.reschedule_timer();
    }

    fn on_tick(&self) {
        let expired = {
            let mut state = self.ivars().state.borrow_mut();
            match state.inhibitor.poll_expired() {
                Ok(true) => {
                    // The countdown ran out: everything is released, so the
                    // toggles go off too, like pressing Stop All.
                    state.settings.set_all(false);
                    prefs::save(&state.settings);
                    true
                }
                Ok(false) => false,
                Err(err) => {
                    state.last_error = Some(err.to_string());
                    true
                }
            }
        };
        if expired {
            self.after_state_change();
            return;
        }
        {
            let mut state = self.ivars().state.borrow_mut();
            if let Err(err) = state.inhibitor.refresh_user_activity() {
                state.last_error = Some(err.to_string());
            }
        }
        self.update_header();
        self.update_status_button();
    }

    /// Run a timer only when there is something to do: 1 Hz while a
    /// countdown is showing, otherwise half the user-activity interval.
    fn reschedule_timer(&self) {
        self.stop_timer();
        let interval = {
            let state = self.ivars().state.borrow();
            if !state.inhibitor.is_active() {
                return;
            }
            if state.inhibitor.deadline().is_some() {
                1.0
            } else if state.inhibitor.next_user_activity_due().is_some() {
                Inhibitor::USER_ACTIVITY_INTERVAL.as_secs_f64() / 2.0
            } else {
                return;
            }
        };
        let target: &AnyObject = self;
        // SAFETY: `tick:` is defined above with the matching signature. The
        // timer retains `self`; it is stored in our ivars and invalidated in
        // `stop_timer` before being replaced or on termination.
        let timer = unsafe {
            NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                interval,
                target,
                sel!(tick:),
                None,
                true,
            )
        };
        // Let the kernel coalesce wake-ups with other timers.
        timer.setTolerance(interval * 0.1);
        // SAFETY: `NSRunLoopCommonModes` is a constant exported by Foundation.
        let modes = unsafe { NSRunLoopCommonModes };
        // SAFETY: scheduling on the main run loop from the main thread; common
        // modes keep the countdown ticking while the menu is open.
        unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, modes) };
        *self.ivars().timer.borrow_mut() = Some(timer);
    }

    fn stop_timer(&self) {
        if let Some(timer) = self.ivars().timer.borrow_mut().take() {
            timer.invalidate();
        }
    }

    /// Make every menu item reflect what is actually held.
    fn sync_ui(&self) {
        let ui = &self.ivars().ui;
        let (current, settings, active, error) = {
            let state = self.ivars().state.borrow();
            (
                state.inhibitor.current(),
                state.settings,
                state.inhibitor.is_active(),
                state.last_error.clone(),
            )
        };

        for (kind, item) in &ui.kind_items {
            item.setState(state_value(current.is_enabled(*kind)));
        }
        ui.user_active_item
            .setState(state_value(current.declare_user_active));

        for item in &ui.timeout_items {
            let secs = u64::try_from(item.tag()).unwrap_or(u64::MAX);
            item.setState(state_value(secs == settings.timeout_secs));
        }
        if is_preset(settings.timeout_secs) {
            ui.custom_item.setTitle(ns_string!("Custom…"));
            ui.custom_item.setState(NSControlStateValueOff);
        } else {
            let label = format!(
                "Custom ({})…",
                format_duration(Duration::from_secs(settings.timeout_secs))
            );
            ui.custom_item.setTitle(&NSString::from_str(&label));
            ui.custom_item.setState(NSControlStateValueOn);
        }

        ui.enable_all_item
            .setEnabled(!(active && settings.all_on()));
        ui.stop_all_item.setEnabled(active);

        match login_item::status() {
            LoginItem::Enabled => {
                ui.login_item.setEnabled(true);
                ui.login_item.setState(NSControlStateValueOn);
                ui.login_item.setToolTip(None);
            }
            LoginItem::Disabled => {
                ui.login_item.setEnabled(true);
                ui.login_item.setState(NSControlStateValueOff);
                ui.login_item.setToolTip(None);
            }
            LoginItem::RequiresApproval => {
                ui.login_item.setEnabled(true);
                ui.login_item.setState(NSControlStateValueMixed);
                ui.login_item.setToolTip(Some(ns_string!(
                    "Approve Bluemeth in System Settings › General › Login Items"
                )));
            }
            LoginItem::Unavailable(why) => {
                ui.login_item.setEnabled(false);
                ui.login_item.setState(NSControlStateValueOff);
                ui.login_item.setToolTip(Some(&NSString::from_str(why)));
            }
        }

        match error {
            Some(err) => {
                ui.error_item
                    .setTitle(&NSString::from_str(&format!("⚠︎ {err}")));
                ui.error_item.setHidden(false);
            }
            None => ui.error_item.setHidden(true),
        }

        self.update_header();
        self.update_status_button();
    }

    fn update_header(&self) {
        let (active, remaining) = {
            let state = self.ivars().state.borrow();
            (state.inhibitor.is_active(), state.inhibitor.remaining())
        };
        let title = match (active, remaining) {
            (false, _) => format!("{APP_NAME}: sleep allowed"),
            (true, None) => format!("{APP_NAME}: keeping awake"),
            (true, Some(left)) => {
                format!("{APP_NAME}: keeping awake, {} left", countdown(left))
            }
        };
        self.ivars().ui.header.setTitle(&NSString::from_str(&title));
    }

    fn update_status_button(&self) {
        let ui = &self.ivars().ui;
        let Some(button) = ui.status_item.button(self.mtm()) else {
            return;
        };
        let (active, remaining, held) = {
            let state = self.ivars().state.borrow();
            let current = state.inhibitor.current();
            let held: Vec<&str> = MENU_KINDS
                .iter()
                .filter(|k| current.is_enabled(**k))
                .map(|k| k.label().trim_start_matches("Prevent "))
                .chain(current.declare_user_active.then_some("User Active"))
                .collect();
            (
                state.inhibitor.is_active(),
                state.inhibitor.remaining(),
                held.join(", "),
            )
        };

        let image = if active {
            &ui.image_active
        } else {
            &ui.image_idle
        };
        button.setImage(image.as_deref());

        if let Some(left) = remaining.filter(|_| active) {
            button.setTitle(&NSString::from_str(&countdown(left)));
            button.setImagePosition(NSCellImagePosition::ImageLeft);
        } else {
            button.setTitle(ns_string!(""));
            button.setImagePosition(NSCellImagePosition::ImageOnly);
        }

        let tooltip = if active {
            format!("{APP_NAME}: keeping awake ({held})")
        } else {
            format!("{APP_NAME}: sleep allowed")
        };
        button.setToolTip(Some(&NSString::from_str(&tooltip)));
    }

    /// Ask for a duration. Returns seconds, or `None` if cancelled.
    fn prompt_custom_timeout(&self) -> Option<u64> {
        let mtm = self.mtm();
        NSApplication::sharedApplication(mtm).activate();

        let alert = NSAlert::new(mtm);
        alert.setMessageText(ns_string!("Custom timeout"));
        alert.setInformativeText(ns_string!(
            "Seconds, or a duration such as 90s, 45m or 1h30m."
        ));
        alert.addButtonWithTitle(ns_string!("Set"));
        alert.addButtonWithTitle(ns_string!("Cancel"));

        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(220.0, 24.0));
        let field = NSTextField::initWithFrame(NSTextField::alloc(mtm), frame);
        field.setPlaceholderString(Some(ns_string!("1h30m")));
        let current = self.ivars().state.borrow().settings.timeout_secs;
        if current > 0 && !is_preset(current) {
            let text = format_duration(Duration::from_secs(current)).replace(' ', "");
            field.setStringValue(&NSString::from_str(&text));
        }
        alert.setAccessoryView(Some(&field));
        alert.window().setInitialFirstResponder(Some(&field));

        loop {
            if alert.runModal() != NSAlertFirstButtonReturn {
                return None;
            }
            let text = field.stringValue().to_string();
            match parse_duration(&text) {
                Ok(duration) if duration > Duration::ZERO => {
                    return Some(duration.as_secs().max(1));
                }
                Ok(_) => {
                    alert.setInformativeText(ns_string!("The timeout must be longer than zero."));
                }
                Err(err) => alert.setInformativeText(&NSString::from_str(&err.to_string())),
            }
        }
    }

    fn show_error(&self, title: &str, detail: &str) {
        let mtm = self.mtm();
        NSApplication::sharedApplication(mtm).activate();
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Warning);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(detail));
        alert.addButtonWithTitle(ns_string!("OK"));
        alert.runModal();
    }
}

fn state_value(on: bool) -> NSControlStateValue {
    if on {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    }
}

fn menu_item(mtm: MainThreadMarker, title: &str, action: Option<Sel>) -> Retained<NSMenuItem> {
    menu_item_with_key(mtm, title, action, "")
}

fn menu_item_with_key(
    mtm: MainThreadMarker,
    title: &str,
    action: Option<Sel>,
    key: &str,
) -> Retained<NSMenuItem> {
    // SAFETY: the designated NSMenuItem initialiser. The action, when given,
    // names a method defined on `Controller`, which `install_targets` makes
    // the target before the menu can be shown.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    }
}

fn symbol_image(name: &str) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(ns_string!("Bluemeth")),
    )?;
    image.setTemplate(true);
    Some(image)
}

fn build_ui(mtm: MainThreadMarker) -> Ui {
    let status_item =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    let image_active = symbol_image(SYMBOL_ACTIVE);
    let image_idle = symbol_image(SYMBOL_IDLE);
    if let Some(button) = status_item.button(mtm) {
        button.setImage(image_idle.as_deref());
        button.setImagePosition(NSCellImagePosition::ImageOnly);
        // Monospaced digits stop the countdown from jittering sideways.
        let size = NSFont::menuBarFontOfSize(0.0).pointSize();
        // SAFETY: `NSFontWeightRegular` is a constant exported by AppKit.
        let weight = unsafe { NSFontWeightRegular };
        button.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
            size, weight,
        )));
    }

    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);

    let header = menu_item(mtm, APP_NAME, None);
    header.setEnabled(false);
    menu.addItem(&header);
    let error_item = menu_item(mtm, "", None);
    error_item.setEnabled(false);
    error_item.setHidden(true);
    menu.addItem(&error_item);
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let mut kind_items = Vec::with_capacity(MENU_KINDS.len());
    for (index, kind) in MENU_KINDS.iter().enumerate() {
        let item = menu_item(mtm, kind.label(), Some(sel!(toggleKind:)));
        item.setTag(index as isize);
        item.setToolTip(Some(&NSString::from_str(kind.description())));
        menu.addItem(&item);
        kind_items.push((*kind, item));
    }
    let user_active_item = menu_item(mtm, "Declare User Active", Some(sel!(toggleUserActive:)));
    user_active_item.setToolTip(Some(ns_string!(
        "Simulates user activity every 30 s: turns the display on and resets the idle timer"
    )));
    menu.addItem(&user_active_item);
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let timeout_item = menu_item(mtm, "Timeout", None);
    let timeout_menu = NSMenu::new(mtm);
    timeout_menu.setAutoenablesItems(false);
    let mut timeout_items = Vec::with_capacity(PRESETS.len());
    for preset in PRESETS {
        let item = menu_item(mtm, preset.label, Some(sel!(selectTimeout:)));
        item.setTag(isize::try_from(preset.secs).unwrap_or(isize::MAX));
        timeout_menu.addItem(&item);
        timeout_items.push(item);
    }
    timeout_menu.addItem(&NSMenuItem::separatorItem(mtm));
    let custom_item = menu_item(mtm, "Custom…", Some(sel!(selectTimeout:)));
    custom_item.setTag(CUSTOM_TAG);
    timeout_menu.addItem(&custom_item);
    timeout_item.setSubmenu(Some(&timeout_menu));
    menu.addItem(&timeout_item);
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let enable_all_item = menu_item(mtm, "Enable All", Some(sel!(enableAll:)));
    menu.addItem(&enable_all_item);
    let stop_all_item = menu_item(mtm, "Stop All", Some(sel!(stopAll:)));
    menu.addItem(&stop_all_item);
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let login_item = menu_item(mtm, "Launch at Login", Some(sel!(toggleLaunchAtLogin:)));
    menu.addItem(&login_item);
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let quit_item = menu_item_with_key(mtm, "Quit Bluemeth", Some(sel!(quit:)), "q");
    menu.addItem(&quit_item);

    status_item.setMenu(Some(&menu));

    Ui {
        status_item,
        image_active,
        image_idle,
        menu,
        header,
        error_item,
        kind_items,
        user_active_item,
        timeout_items,
        custom_item,
        enable_all_item,
        stop_all_item,
        login_item,
        quit_item,
    }
}

/// Build the UI and run the AppKit main loop. Never returns.
pub fn run() {
    let mtm = MainThreadMarker::new().expect("Bluemeth must start on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon, no menu bar of our own: a pure status item app.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let state = State {
        settings: prefs::load(),
        inhibitor: Inhibitor::new(APP_NAME),
        last_error: None,
    };
    let ui = build_ui(mtm);
    let controller = Controller::new(mtm, ui, state);
    controller.install_targets();
    app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));

    #[cfg(debug_assertions)]
    debug_hooks(&controller);

    app.run();
}

/// Debug builds only: `BLUEMETH_DEBUG_OPEN_MENU=1` opens the menu shortly
/// after launch so the UI can be screenshotted from a script.
#[cfg(debug_assertions)]
fn debug_hooks(controller: &Controller) {
    if std::env::var_os("BLUEMETH_DEBUG_OPEN_MENU").is_none() {
        return;
    }
    let target: &AnyObject = controller;
    // SAFETY: `openMenu:` is defined on `Controller`; a one-shot timer retains
    // its target until it fires, and the controller outlives it anyway.
    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            0.8,
            target,
            sel!(openMenu:),
            None,
            false,
        )
    };
    // SAFETY: see `reschedule_timer`.
    unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
}
