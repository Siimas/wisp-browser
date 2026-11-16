//! EXPERIMENTAL — extensions spike, see docs/EXTENSIONS-SPIKE.md. Not a feature.
//!
//! Loads browser extensions into tab webviews through WebKit's own web-extension
//! support (`WKWebExtension*`, macOS 15.4+). Does nothing unless the environment
//! variable `WISP_EXTENSIONS` lists extensions separated by `:` (an unpacked folder,
//! a .zip, or an `.appex` Safari web extension bundle).
//!
//! ponytail: every permission an extension asks for is granted without asking, and
//! everything is logged to stderr with a `wisp-ext:` prefix. Test builds only.
//! Private tabs never get the controller, so they stay extension-free.

use block2::{DynBlock, RcBlock};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
};
use objc2_foundation::{
    NSArray, NSBundle, NSDate, NSError, NSPoint, NSRect, NSSet, NSSize, NSString, NSUInteger, NSURL,
};
use objc2_web_kit::{
    WKWebExtension, WKWebExtensionAction, WKWebExtensionContext,
    WKWebExtensionContextPermissionStatus, WKWebExtensionController,
    WKWebExtensionControllerConfiguration, WKWebExtensionControllerDelegate,
    WKWebExtensionMatchPattern, WKWebExtensionMessagePort, WKWebExtensionPermission,
    WKWebExtensionTab, WKWebExtensionTabConfiguration, WKWebExtensionWindow,
    WKWebExtensionWindowType, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
use std::{cell::RefCell, ffi::c_void, ptr::NonNull, time::Instant};
use wry::{WebView, WebViewExtMacOS};

struct State {
    controller: Retained<WKWebExtensionController>,
    _delegate: Retained<Delegate>,
    window: Retained<ExtWindow>,
    tabs: Vec<Retained<ExtTab>>,
    active: Option<u32>,
    contexts: Vec<Retained<WKWebExtensionContext>>,
    ns_window: *mut c_void,
    popup: Option<Retained<WKWebView>>,
    started: Instant,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn log(msg: impl AsRef<str>) {
    let ms = STATE.with_borrow(|s| s.as_ref().map(|s| s.started.elapsed().as_millis()));
    eprintln!("wisp-ext: [{:>5}ms] {}", ms.unwrap_or(0), msg.as_ref());
}

fn error(text: &str) -> Retained<NSError> {
    log(format!("  -> answering with error: {text}"));
    // ponytail: the message goes in the domain; good enough for a log line in the extension.
    NSError::new(1, &NSString::from_str(&format!("Wisp: {text}")))
}

fn describe(obj: &AnyObject) -> String {
    // SAFETY: every NSObject answers -description.
    let d: Retained<NSString> = unsafe { msg_send![obj, description] };
    d.to_string().replace('\n', " ")
}

fn window_obj() -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
    STATE.with_borrow(|s| {
        s.as_ref()
            .map(|s| ProtocolObject::from_retained(s.window.clone()))
    })
}

fn tab_obj(id: u32) -> Option<Retained<ExtTab>> {
    STATE.with_borrow(|s| {
        s.as_ref()?
            .tabs
            .iter()
            .find(|t| t.ivars().id == id)
            .cloned()
    })
}

pub struct TabIvars {
    id: u32,
    web: Retained<WKWebView>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WispExtTab"]
    #[ivars = TabIvars]
    struct ExtTab;

    unsafe impl NSObjectProtocol for ExtTab {}

    unsafe impl WKWebExtensionTab for ExtTab {
        #[unsafe(method_id(windowForWebExtensionContext:))]
        fn window_for(
            &self,
            _x: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            window_obj()
        }

        #[unsafe(method(indexInWindowForWebExtensionContext:))]
        fn index_for(&self, _x: &WKWebExtensionContext) -> NSUInteger {
            let id = self.ivars().id;
            STATE.with_borrow(|s| {
                s.as_ref()
                    .and_then(|s| s.tabs.iter().position(|t| t.ivars().id == id))
                    .unwrap_or(0)
            })
        }

        #[unsafe(method_id(webViewForWebExtensionContext:))]
        fn web_view_for(&self, _x: &WKWebExtensionContext) -> Option<Retained<WKWebView>> {
            Some(self.ivars().web.clone())
        }

        #[unsafe(method_id(titleForWebExtensionContext:))]
        fn title_for(&self, _x: &WKWebExtensionContext) -> Option<Retained<NSString>> {
            // SAFETY: main-thread property read on a live web view.
            unsafe { self.ivars().web.title() }
        }

        #[unsafe(method_id(urlForWebExtensionContext:))]
        fn url_for(&self, _x: &WKWebExtensionContext) -> Option<Retained<NSURL>> {
            // SAFETY: as above.
            unsafe { self.ivars().web.URL() }
        }

        #[unsafe(method(isSelectedForWebExtensionContext:))]
        fn is_selected(&self, _x: &WKWebExtensionContext) -> bool {
            let id = self.ivars().id;
            STATE.with_borrow(|s| s.as_ref().and_then(|s| s.active) == Some(id))
        }

        #[unsafe(method(isLoadingCompleteForWebExtensionContext:))]
        fn is_loaded(&self, _x: &WKWebExtensionContext) -> bool {
            // SAFETY: as above.
            !unsafe { self.ivars().web.isLoading() }
        }
    }
);

define_class!(
    // SAFETY: as above.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WispExtWindow"]
    struct ExtWindow;

    unsafe impl NSObjectProtocol for ExtWindow {}

    unsafe impl WKWebExtensionWindow for ExtWindow {
        #[unsafe(method_id(tabsForWebExtensionContext:))]
        fn tabs_for(
            &self,
            _x: &WKWebExtensionContext,
        ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
            let tabs: Vec<_> = STATE.with_borrow(|s| {
                s.as_ref()
                    .map(|s| {
                        s.tabs
                            .iter()
                            .cloned()
                            .map(ProtocolObject::from_retained)
                            .collect()
                    })
                    .unwrap_or_default()
            });
            NSArray::from_retained_slice(&tabs)
        }

        #[unsafe(method_id(activeTabForWebExtensionContext:))]
        fn active_tab_for(
            &self,
            _x: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
            STATE
                .with_borrow(|s| s.as_ref().and_then(|s| s.active))
                .and_then(tab_obj)
                .map(ProtocolObject::from_retained)
        }

        #[unsafe(method(windowTypeForWebExtensionContext:))]
        fn type_for(&self, _x: &WKWebExtensionContext) -> WKWebExtensionWindowType {
            WKWebExtensionWindowType::Normal
        }

        #[unsafe(method(isPrivateForWebExtensionContext:))]
        fn is_private(&self, _x: &WKWebExtensionContext) -> bool {
            false
        }
    }
);

type Ctl = WKWebExtensionController;
type Ctx = WKWebExtensionContext;
type TabArg<'a> = Option<&'a ProtocolObject<dyn WKWebExtensionTab>>;

define_class!(
    // SAFETY: as above.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WispExtDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl WKWebExtensionControllerDelegate for Delegate {
        #[unsafe(method_id(webExtensionController:openWindowsForExtensionContext:))]
        fn open_windows(
            &self,
            _c: &Ctl,
            _x: &Ctx,
        ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionWindow>>> {
            NSArray::from_retained_slice(&window_obj().into_iter().collect::<Vec<_>>())
        }

        #[unsafe(method_id(webExtensionController:focusedWindowForExtensionContext:))]
        fn focused_window(
            &self,
            _c: &Ctl,
            _x: &Ctx,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            window_obj()
        }

        #[unsafe(method(webExtensionController:openNewTabUsingConfiguration:forExtensionContext:completionHandler:))]
        fn open_tab(
            &self,
            _c: &Ctl,
            cfg: &WKWebExtensionTabConfiguration,
            x: &Ctx,
            done: &DynBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError)>,
        ) {
            // SAFETY: property read.
            let url = unsafe { cfg.url() }
                .map(|u| describe(&u))
                .unwrap_or_default();
            log(format!("{} asked to open a tab: {url}", name(x)));
            done.call((
                std::ptr::null_mut(),
                Retained::as_ptr(&error("tabs.create not wired in the spike")) as *mut _,
            ));
        }

        #[unsafe(method(webExtensionController:openOptionsPageForExtensionContext:completionHandler:))]
        fn open_options(&self, _c: &Ctl, x: &Ctx, done: &DynBlock<dyn Fn(*mut NSError)>) {
            log(format!("{} asked to open its options page", name(x)));
            done.call((Retained::as_ptr(&error("options page not wired")) as *mut _,));
        }

        #[unsafe(method(webExtensionController:promptForPermissions:inTab:forExtensionContext:completionHandler:))]
        fn prompt_permissions(
            &self,
            _c: &Ctl,
            perms: &NSSet<WKWebExtensionPermission>,
            _t: TabArg<'_>,
            x: &Ctx,
            done: &DynBlock<dyn Fn(NonNull<NSSet<WKWebExtensionPermission>>, *mut NSDate)>,
        ) {
            log(format!(
                "{} prompts for permissions {} -> granted (spike)",
                name(x),
                describe(perms)
            ));
            done.call((NonNull::from(perms), std::ptr::null_mut()));
        }

        #[unsafe(method(webExtensionController:promptForPermissionToAccessURLs:inTab:forExtensionContext:completionHandler:))]
        fn prompt_urls(
            &self,
            _c: &Ctl,
            urls: &NSSet<NSURL>,
            _t: TabArg<'_>,
            x: &Ctx,
            done: &DynBlock<dyn Fn(NonNull<NSSet<NSURL>>, *mut NSDate)>,
        ) {
            log(format!(
                "{} prompts for URLs {} -> granted (spike)",
                name(x),
                describe(urls)
            ));
            done.call((NonNull::from(urls), std::ptr::null_mut()));
        }

        #[unsafe(method(webExtensionController:promptForPermissionMatchPatterns:inTab:forExtensionContext:completionHandler:))]
        fn prompt_patterns(
            &self,
            _c: &Ctl,
            pats: &NSSet<WKWebExtensionMatchPattern>,
            _t: TabArg<'_>,
            x: &Ctx,
            done: &DynBlock<dyn Fn(NonNull<NSSet<WKWebExtensionMatchPattern>>, *mut NSDate)>,
        ) {
            log(format!(
                "{} prompts for patterns {} -> granted (spike)",
                name(x),
                describe(pats)
            ));
            done.call((NonNull::from(pats), std::ptr::null_mut()));
        }

        #[unsafe(method(webExtensionController:didUpdateAction:forExtensionContext:))]
        fn did_update_action(&self, _c: &Ctl, action: &WKWebExtensionAction, x: &Ctx) {
            // SAFETY: property reads.
            let (label, badge) = unsafe { (action.label(), action.badgeText()) };
            log(format!(
                "{} action updated: label={label:?} badge={badge:?}",
                name(x)
            ));
        }

        #[unsafe(method(webExtensionController:presentPopupForAction:forExtensionContext:completionHandler:))]
        fn present_popup(
            &self,
            _c: &Ctl,
            action: &WKWebExtensionAction,
            x: &Ctx,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            log(format!("{} presents its toolbar popup", name(x)));
            // SAFETY: property read; the web view is shown on the main thread.
            match unsafe { action.popupWebView() } {
                Some(web) => {
                    show_popup(&web);
                    // Spike diagnostics: record the popup's errors, then reload it so the
                    // recorder is in place from the first script on.
                    if std::env::var_os("WISP_POPUP_PROBE").is_some() {
                        unsafe {
                            let mtm = MainThreadMarker::new().expect("main thread");
                            let script = objc2_web_kit::WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                                objc2_web_kit::WKUserScript::alloc(mtm),
                                &NSString::from_str(ERROR_RECORDER),
                                objc2_web_kit::WKUserScriptInjectionTime::AtDocumentStart,
                                false,
                            );
                            web.configuration()
                                .userContentController()
                                .addUserScript(&script);
                            web.reload();
                        }
                    }
                    STATE.with_borrow_mut(|s| s.as_mut().map(|s| s.popup = Some(web.clone())));
                    done.call((std::ptr::null_mut(),));
                }
                None => done.call((Retained::as_ptr(&error("no popup web view")) as *mut _,)),
            }
        }

        #[unsafe(method(webExtensionController:sendMessage:toApplicationWithIdentifier:forExtensionContext:replyHandler:))]
        fn native_message(
            &self,
            _c: &Ctl,
            message: &AnyObject,
            app: Option<&NSString>,
            x: &Ctx,
            reply: &DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>,
        ) {
            log(format!(
                "{} NATIVE MESSAGE to app {:?}: {}",
                name(x),
                app.map(|a| a.to_string()),
                describe(message).chars().take(300).collect::<String>()
            ));
            reply.call((
                std::ptr::null_mut(),
                Retained::as_ptr(&error("no native app host in Wisp")) as *mut _,
            ));
        }

        #[unsafe(method(webExtensionController:connectUsingMessagePort:forExtensionContext:completionHandler:))]
        fn native_connect(
            &self,
            _c: &Ctl,
            port: &WKWebExtensionMessagePort,
            x: &Ctx,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            // SAFETY: property read.
            let app = unsafe { port.applicationIdentifier() };
            log(format!(
                "{} NATIVE CONNECT (port) to app {:?}",
                name(x),
                app.map(|a| a.to_string())
            ));
            done.call((Retained::as_ptr(&error("no native app host in Wisp")) as *mut _,));
        }
    }
);

fn name(x: &Ctx) -> String {
    // SAFETY: property read.
    unsafe { x.webExtension().displayName() }
        .map(|n| n.to_string())
        .unwrap_or_default()
}

/// Put the extension's popup web view in the window's top-right corner (the spike's
/// stand-in for a toolbar button's popover).
fn show_popup(web: &WKWebView) {
    let Some(ns_window) = STATE.with_borrow(|s| s.as_ref().map(|s| s.ns_window)) else {
        return;
    };
    // SAFETY: main-thread AppKit calls on live views.
    unsafe {
        let content: *mut AnyObject = msg_send![ns_window as *mut AnyObject, contentView];
        let bounds: NSRect = msg_send![content, bounds];
        let (w, h) = (360.0, 480.0);
        let frame = NSRect::new(
            NSPoint::new(bounds.size.width - w - 16.0, bounds.size.height - h - 56.0),
            NSSize::new(w, h),
        );
        let _: () = msg_send![web, setFrame: frame];
        let _: () = msg_send![web, setWantsLayer: true];
        let layer: *mut AnyObject = msg_send![web, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setCornerRadius: 12.0f64];
            let _: () = msg_send![layer, setMasksToBounds: true];
            let _: () = msg_send![layer, setBorderWidth: 1.0f64];
        }
        let _: () = msg_send![content, addSubview: web];
    }
}

/// Start the controller and begin loading every extension in `WISP_EXTENSIONS`.
pub fn init(ns_window: *mut c_void) {
    let Some(list) = std::env::var_os("WISP_EXTENSIONS") else {
        return;
    };
    let mtm = MainThreadMarker::new().expect("main thread");
    let started = Instant::now();
    // SAFETY: WebKit object creation on the main thread; classes exist on macOS 15.4+.
    unsafe {
        let config = WKWebExtensionControllerConfiguration::defaultConfiguration(mtm);
        // Extension pages and backgrounds get their web views from this base. Extensions
        // that pick their code path from the user agent (Bitwarden) need " Safari/" in
        // it, as tabs already have (native::user_agent).
        let base = config.webViewConfiguration();
        if std::env::var_os("WISP_EXT_PLAIN_UA").is_none() {
            let safari = crate::native::user_agent()
                .and_then(|ua| {
                    ua.split(") ")
                        .last()
                        .map(|s| s.replace("(KHTML, like Gecko) ", ""))
                })
                .unwrap_or_else(|| "Version/26.0 Safari/605.1.15".into());
            base.setApplicationNameForUserAgent(Some(&NSString::from_str(&safari)));
        }
        let script = objc2_web_kit::WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
            objc2_web_kit::WKUserScript::alloc(mtm),
            &NSString::from_str(ERROR_RECORDER),
            objc2_web_kit::WKUserScriptInjectionTime::AtDocumentStart,
            false,
        );
        base.userContentController().addUserScript(&script);
        config.setWebViewConfiguration(Some(&base));
        let controller = WKWebExtensionController::initWithConfiguration(
            WKWebExtensionController::alloc(mtm),
            &config,
        );
        let delegate: Retained<Delegate> =
            msg_send![super(Delegate::alloc(mtm).set_ivars(())), init];
        controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let window: Retained<ExtWindow> =
            msg_send![super(ExtWindow::alloc(mtm).set_ivars(())), init];
        STATE.set(Some(State {
            controller: controller.clone(),
            _delegate: delegate,
            window: window.clone(),
            tabs: Vec::new(),
            active: None,
            contexts: Vec::new(),
            ns_window,
            popup: None,
            started,
        }));
        controller.didOpenWindow(ProtocolObject::from_ref(&*window));
        controller.didFocusWindow(Some(ProtocolObject::from_ref(&*window)));
    }
    log("controller ready");
    for path in std::env::split_paths(&list) {
        let path_s = path.to_string_lossy().to_string();
        let done = RcBlock::new(move |ext: *mut WKWebExtension, err: *mut NSError| {
            // SAFETY: WebKit hands us either an extension or an error, retained by the caller.
            match unsafe { ext.as_ref() } {
                Some(ext) => load(ext),
                None => log(format!(
                    "FAILED to read {path_s}: {}",
                    unsafe { err.as_ref() }
                        .map(|e| describe(e))
                        .unwrap_or_default()
                )),
            }
        });
        // SAFETY: as above; the completion runs later on the main thread.
        unsafe {
            if path.extension().is_some_and(|e| e == "appex") {
                let Some(bundle) =
                    NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))
                else {
                    log(format!("not a bundle: {}", path.display()));
                    continue;
                };
                WKWebExtension::extensionWithAppExtensionBundle_completionHandler(
                    &bundle, &done, mtm,
                );
            } else {
                let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &done, mtm);
            }
        }
    }
}

const ERROR_RECORDER: &str = "window.__wispErrors=[];\
addEventListener('error',e=>__wispErrors.push('error: '+e.message+' @'+(e.filename||'').split('/').pop()+':'+e.lineno));\
addEventListener('unhandledrejection',e=>__wispErrors.push('rejection: '+(e.reason&&(e.reason.stack||e.reason.message)||e.reason)));\
const __ce=console.error.bind(console);console.error=(...a)=>{__wispErrors.push('console.error: '+a.map(x=>x&&x.stack?(x.name+': '+x.message+' | '+x.stack.split('\\n')[0]):String(x)).join(' ').slice(0,400));__ce(...a)};\
const __cw=console.warn.bind(console);console.warn=(...a)=>{__wispErrors.push('console.warn: '+a.map(String).join(' ').slice(0,300));__cw(...a)};";

fn load(ext: &WKWebExtension) {
    let mtm = MainThreadMarker::new().expect("main thread");
    // SAFETY: main-thread WebKit calls on live objects.
    unsafe {
        let name = ext.displayName().map(|n| n.to_string()).unwrap_or_default();
        log(format!(
            "read \"{name}\" {} (manifest v{}) background={} persistent={} content={} options={} commands={} dnr-rules={}",
            ext.version().map(|v| v.to_string()).unwrap_or_default(),
            ext.manifestVersion(),
            ext.hasBackgroundContent(),
            ext.hasPersistentBackgroundContent(),
            ext.hasInjectedContent(),
            ext.hasOptionsPage(),
            ext.hasCommands(),
            ext.hasContentModificationRules(),
        ));
        for e in ext.errors().iter() {
            log(format!("  manifest/parse problem: {}", describe(&e)));
        }
        log(format!(
            "  requested permissions: {}",
            describe(&ext.requestedPermissions())
        ));
        log(format!(
            "  optional permissions: {}",
            describe(&ext.optionalPermissions())
        ));
        log(format!(
            "  host patterns: {}",
            describe(&ext.allRequestedMatchPatterns())
        ));

        let ctx = WKWebExtensionContext::contextForExtension(ext);
        ctx.setInspectable(true);
        // Spike diagnostics: record errors in extension pages (popup, options) so the
        // popup probe can read them. Does not reach service-worker backgrounds.
        if let Some(config) = ctx.webViewConfiguration() {
            let src = NSString::from_str(ERROR_RECORDER);
            let script = objc2_web_kit::WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                objc2_web_kit::WKUserScript::alloc(mtm),
                &src,
                objc2_web_kit::WKUserScriptInjectionTime::AtDocumentStart,
                false,
            );
            config.userContentController().addUserScript(&script);
        }
        // ponytail: grant everything requested, as a user clicking "Allow" on all would.
        for p in ext.requestedPermissions().iter() {
            ctx.setPermissionStatus_forPermission(
                WKWebExtensionContextPermissionStatus::GrantedExplicitly,
                &p,
            );
        }
        for p in ext.allRequestedMatchPatterns().iter() {
            ctx.setPermissionStatus_forMatchPattern(
                WKWebExtensionContextPermissionStatus::GrantedExplicitly,
                &p,
            );
        }
        let _ = mtm;
        let Some(controller) = STATE.with_borrow(|s| s.as_ref().map(|s| s.controller.clone()))
        else {
            return;
        };
        let t = Instant::now();
        if let Err(e) = controller.loadExtensionContext_error(&ctx) {
            log(format!("  FAILED to load \"{name}\": {}", describe(&e)));
            return;
        }
        log(format!(
            "  loaded \"{name}\" in {} ms; base {}",
            t.elapsed().as_millis(),
            describe(&ctx.baseURL())
        ));
        for e in ctx.errors().iter() {
            log(format!("  context problem: {}", describe(&e)));
        }
        STATE.with_borrow_mut(|s| {
            if let Some(s) = s {
                s.contexts.push(ctx.clone());
            }
        });
        if ext.hasBackgroundContent() {
            let (n, t) = (name.clone(), Instant::now());
            let done = RcBlock::new(move |err: *mut NSError| match err.as_ref() {
                None => log(format!(
                    "  background of \"{n}\" running ({} ms)",
                    t.elapsed().as_millis()
                )),
                Some(e) => log(format!("  background of \"{n}\" FAILED: {}", describe(e))),
            });
            ctx.loadBackgroundContentWithCompletionHandler(&done);
        }
    }
}

/// A configuration carrying the extension controller, for a new non-private tab.
/// `None` when extensions are off: wry then builds its own, as before.
pub fn configuration(private: bool) -> Option<Retained<WKWebViewConfiguration>> {
    if private {
        return None;
    }
    let controller = STATE.with_borrow(|s| s.as_ref().map(|s| s.controller.clone()))?;
    let mtm = MainThreadMarker::new()?;
    // SAFETY: main-thread WebKit object creation. wry skips its own data-store choice
    // when handed a configuration, so set the default (persistent) one here.
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        config.setWebExtensionController(Some(&controller));
        config.setWebsiteDataStore(&WKWebsiteDataStore::defaultDataStore(mtm));
        Some(config)
    }
}

pub fn did_open_tab(id: u32, webview: &WebView) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(controller) = STATE.with_borrow(|s| s.as_ref().map(|s| s.controller.clone())) else {
        return;
    };
    let web: Retained<WKWebView> = Retained::into_super(webview.webview());
    // SAFETY: the configuration getter returns a copy; reading its controller is safe.
    if unsafe { web.configuration().webExtensionController() }.is_none() {
        return; // private tab: built without the controller
    }
    let tab: Retained<ExtTab> = unsafe {
        msg_send![
            super(ExtTab::alloc(mtm).set_ivars(TabIvars { id, web })),
            init
        ]
    };
    STATE.with_borrow_mut(|s| s.as_mut().map(|s| s.tabs.push(tab.clone())));
    // SAFETY: main-thread notification to the controller.
    unsafe { controller.didOpenTab(ProtocolObject::from_ref(&*tab)) };
}

pub fn did_activate(id: u32) {
    let Some(controller) = STATE.with_borrow(|s| s.as_ref().map(|s| s.controller.clone())) else {
        return;
    };
    let prev = STATE.with_borrow_mut(|s| s.as_mut().and_then(|s| s.active.replace(id)));
    let (Some(tab), prev) = (tab_obj(id), prev.and_then(tab_obj)) else {
        return;
    };
    // SAFETY: as above.
    unsafe {
        controller.didActivateTab_previousActiveTab(
            ProtocolObject::from_ref(&*tab),
            prev.as_deref().map(ProtocolObject::from_ref),
        )
    };
}

pub fn did_close_tab(id: u32) {
    let Some(controller) = STATE.with_borrow(|s| s.as_ref().map(|s| s.controller.clone())) else {
        return;
    };
    let Some(tab) = tab_obj(id) else { return };
    STATE.with_borrow_mut(|s| s.as_mut().map(|s| s.tabs.retain(|t| t.ivars().id != id)));
    // SAFETY: as above.
    unsafe { controller.didCloseTab_windowIsClosing(ProtocolObject::from_ref(&*tab), false) };
}

/// Debug hook: press the toolbar button of every loaded extension for the active tab.
pub fn press_actions() {
    let (contexts, active) = STATE.with_borrow(|s| {
        s.as_ref()
            .map(|s| (s.contexts.clone(), s.active))
            .unwrap_or_default()
    });
    let tab = active.and_then(tab_obj);
    for ctx in contexts {
        // SAFETY: property reads.
        for e in unsafe { ctx.errors() }.iter() {
            log(format!(
                "  later context problem in \"{}\": {}",
                name(&ctx),
                describe(&e)
            ));
        }
        log(format!("pressing the toolbar button of \"{}\"", name(&ctx)));
        // SAFETY: main-thread call on a loaded context.
        unsafe { ctx.performActionForTab(tab.as_deref().map(ProtocolObject::from_ref)) };
    }
}

/// Debug hook: run `js` in the open extension popup and print the result.
pub fn probe_popup(js: &str) {
    let Some(web) = STATE.with_borrow(|s| s.as_ref().and_then(|s| s.popup.clone())) else {
        return eprintln!("wisp-popup-probe: no popup open");
    };
    let done = RcBlock::new(|r: *mut AnyObject, e: *mut NSError| {
        // SAFETY: WebKit passes a result object or an error, valid for this call.
        let out = unsafe {
            r.as_ref()
                .map(describe)
                .or_else(|| e.as_ref().map(|e| describe(e)))
        };
        eprintln!("wisp-popup-probe: {}", out.unwrap_or_default());
    });
    // SAFETY: main-thread call on a live web view.
    unsafe { web.evaluateJavaScript_completionHandler(&NSString::from_str(js), Some(&done)) };
}
