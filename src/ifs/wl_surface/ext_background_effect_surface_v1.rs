use crate::client::Client;
use crate::client::ClientError;
use crate::ifs::wl_surface::WlSurface;
use crate::leaks::Tracker;
use crate::object::Object;
use crate::object::Version;
use crate::wire::ExtBackgroundEffectSurfaceV1Id;
use crate::wire::ext_background_effect_surface_v1::*;
use std::rc::Rc;
use thiserror::Error;

const ERROR_SURFACE_DESTROYED: u32 = 0;

pub struct ExtBackgroundEffectSurfaceV1 {
    pub id: ExtBackgroundEffectSurfaceV1Id,
    pub client: Rc<Client>,
    pub surface: Rc<WlSurface>,
    pub tracker: Tracker<Self>,
    pub version: Version,
}

impl ExtBackgroundEffectSurfaceV1 {
    pub fn new(
        id: ExtBackgroundEffectSurfaceV1Id,
        version: Version,
        surface: &Rc<WlSurface>,
    ) -> Self {
        Self {
            id,
            client: surface.client.clone(),
            surface: surface.clone(),
            tracker: Default::default(),
            version,
        }
    }
}

impl ExtBackgroundEffectSurfaceV1RequestHandler for ExtBackgroundEffectSurfaceV1 {
    type Error = ExtBackgroundEffectSurfaceV1Error;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.surface.background_effect.take();
        self.surface.pending.borrow_mut().blur_region = Some(None);
        self.client.remove_obj(self)?;
        Ok(())
    }

    fn set_blur_region(&self, req: SetBlurRegion, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        if self.surface.destroyed.get() {
            self.client.protocol_error(
                self,
                ERROR_SURFACE_DESTROYED,
                "wl_surface has been destroyed",
            );
            return Ok(());
        }
        let region = match req.region.is_some() {
            true => Some(self.client.lookup(req.region)?.region()),
            false => None,
        };
        self.surface.pending.borrow_mut().blur_region = Some(region);
        Ok(())
    }
}

object_base! {
    self = ExtBackgroundEffectSurfaceV1;
    version = self.version;
}

impl Object for ExtBackgroundEffectSurfaceV1 {}

simple_add_obj!(ExtBackgroundEffectSurfaceV1);

#[derive(Debug, Error)]
pub enum ExtBackgroundEffectSurfaceV1Error {
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(ExtBackgroundEffectSurfaceV1Error, ClientError);
