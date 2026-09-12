use crate::agent::AgentError;
use block2::{DynBlock, RcBlock};
use objc2::{
    define_class, msg_send, rc::Retained, runtime::ProtocolObject, AnyThread, DefinedClass,
};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::*;
use std::cell::RefCell;

// The center uses a weak delegate. Retain it for the application's main thread lifetime.
thread_local! {static DELEGATE:RefCell<Option<Retained<Delegate>>>=const{RefCell::new(None)};}
define_class!(
    #[unsafe(super=NSObject)]
    #[name = "FoksChatNotificationDelegate"]
    #[ivars=tauri::AppHandle]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn present(
            &self,
            _: &UNUserNotificationCenter,
            n: &UNNotification,
            done: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            let token = n.request().identifier().to_string();
            let allowed = super::super::route_current(self.ivars(), &token);
            done.call((if allowed {
                UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List
            } else {
                UNNotificationPresentationOptions::empty()
            },));
        }
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn response(
            &self,
            _: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            done: &DynBlock<dyn Fn()>,
        ) {
            super::super::activate(
                self.ivars(),
                &response.notification().request().identifier().to_string(),
            );
            done.call(());
        }
    }
);
pub fn available() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
}
pub fn install(app: &tauri::AppHandle) {
    if !available() {
        return;
    }
    let this = Delegate::alloc().set_ivars(app.clone());
    // SAFETY: NSObject initialization has no additional subclass requirements;
    // Rust ivars are installed before invoking the superclass initializer.
    let delegate: Retained<Delegate> = unsafe { msg_send![super(this), init] };
    UNUserNotificationCenter::currentNotificationCenter()
        .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.with(|slot| *slot.borrow_mut() = Some(delegate));
    // No restart backlog or routing tokens survive the process.
    clear();
}
pub async fn permission() -> bool {
    if !available() {
        return false;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let tx = std::sync::Mutex::new(Some(tx));
        let callback = RcBlock::new(move |granted: objc2::runtime::Bool, _: *mut NSError| {
            if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
                let _ = tx.send(granted.as_bool());
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert,
                &callback,
            );
    }
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(30), rx).await,
        Ok(Ok(true))
    )
}
pub fn display(app: &tauri::AppHandle, token: &str, body: &str) -> Result<(), AgentError> {
    if !available() {
        return Err(super::super::error(
            "Desktop alerts require the packaged macOS application.",
        ));
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str("FOKS"));
    content.setBody(&NSString::from_str(body));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(token),
        &content,
        None,
    );
    let app = app.clone();
    let token = token.to_owned();
    let callback = RcBlock::new(move |error: *mut NSError| {
        if !error.is_null() || !super::super::route_current(&app, &token) {
            let ids = objc2_foundation::NSArray::from_retained_slice(&[NSString::from_str(&token)]);
            let center = UNUserNotificationCenter::currentNotificationCenter();
            center.removePendingNotificationRequestsWithIdentifiers(&ids);
            center.removeDeliveredNotificationsWithIdentifiers(&ids);
            if !error.is_null() {
                super::super::delivery_failed(&app);
            }
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
    Ok(())
}
pub fn clear() {
    if available() {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        center.removeAllPendingNotificationRequests();
        center.removeAllDeliveredNotifications();
    }
}
