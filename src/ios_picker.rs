use std::path::PathBuf;
use std::sync::Mutex;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send};
use objc2_foundation::{NSArray, NSData, NSError, NSString};

use crate::pick_slot::PickSlot;

#[link(name = "PhotosUI", kind = "framework")]
unsafe extern "C" {}

static PENDING: PickSlot = PickSlot::new();
static DELEGATE: Mutex<Option<usize>> = Mutex::new(None);

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ColouringPagesPickerDelegate"]
    struct PickerDelegate;

    impl PickerDelegate {
        #[unsafe(method(picker:didFinishPicking:))]
        fn did_finish(&self, picker: &AnyObject, results: &NSArray<AnyObject>) {
            let _: () = unsafe { msg_send![picker, dismissViewControllerAnimated: true, completion: std::ptr::null::<AnyObject>()] };
            match results.firstObject() {
                Some(result) => load(&result),
                None => PENDING.finish(None),
            }
        }
    }
);

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    fn UIImagePNGRepresentation(image: *mut AnyObject) -> *mut NSData;
}

fn load(result: &AnyObject) {
    let provider: Retained<AnyObject> = unsafe { msg_send![result, itemProvider] };
    let Some(image_class) = AnyClass::get(c"UIImage") else {
        PENDING.finish(None);
        return;
    };
    let block = RcBlock::new(|image: *mut AnyObject, _error: *mut NSError| {
        let bytes = if image.is_null() {
            None
        } else {
            unsafe { UIImagePNGRepresentation(image).as_ref() }.map(|d| d.to_vec())
        };
        PENDING.finish(bytes.and_then(save));
    });
    let _: Retained<AnyObject> =
        unsafe { msg_send![&*provider, loadObjectOfClass: image_class, completionHandler: &*block] };
}

fn save(bytes: Vec<u8>) -> Option<PathBuf> {
    let path = std::env::temp_dir().join("picked-reference");
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

fn present() -> Option<()> {
    let config_class = AnyClass::get(c"PHPickerConfiguration")?;
    let picker_class = AnyClass::get(c"PHPickerViewController")?;
    let filter_class = AnyClass::get(c"PHPickerFilter")?;
    let app_class = AnyClass::get(c"UIApplication")?;
    let config: Retained<AnyObject> = unsafe { msg_send![msg_send![config_class, alloc], init] };
    let filter: Retained<AnyObject> = unsafe { msg_send![filter_class, imagesFilter] };
    let _: () = unsafe { msg_send![&*config, setFilter: &*filter] };
    let _: () = unsafe { msg_send![&*config, setSelectionLimit: 1isize] };
    let picker: Retained<AnyObject> =
        unsafe { msg_send![msg_send![picker_class, alloc], initWithConfiguration: &*config] };
    let delegate: Retained<PickerDelegate> = unsafe { msg_send![PickerDelegate::class(), new] };
    let _: () = unsafe { msg_send![&*picker, setDelegate: &*delegate] };
    *DELEGATE.lock().ok()? = Some(Retained::into_raw(delegate) as usize);
    let app: Retained<AnyObject> = unsafe { msg_send![app_class, sharedApplication] };
    let windows: Retained<NSArray<AnyObject>> = unsafe { msg_send![&*app, windows] };
    let window = windows.firstObject()?;
    let mut top: Retained<AnyObject> = unsafe { msg_send![&*window, rootViewController] };
    loop {
        let next: Option<Retained<AnyObject>> = unsafe { msg_send![&*top, presentedViewController] };
        match next {
            Some(n) => top = n,
            None => break,
        }
    }
    let _: () = unsafe {
        msg_send![&*top, presentViewController: &*picker, animated: true, completion: std::ptr::null::<AnyObject>()]
    };
    Some(())
}

pub fn pick(done: Box<dyn FnOnce(Option<PathBuf>) + Send>) {
    PENDING.set(done);
    dispatch2::DispatchQueue::main().exec_async(|| {
        if present().is_none() {
            PENDING.finish(None);
        }
    });
}
