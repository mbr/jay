use crate::client::Client;
use crate::client::ClientError;
use crate::globals::Global;
use crate::globals::GlobalName;
use crate::ifs::wl_surface::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;
use crate::leaks::Tracker;
use crate::object::Object;
use crate::object::Version;
use crate::wire::ExtBackgroundEffectManagerV1Id;
use crate::wire::ext_background_effect_manager_v1::*;
use std::rc::Rc;
use thiserror::Error;

pub const CAPABILITY_BLUR: u32 = 1;
const ERROR_BACKGROUND_EFFECT_EXISTS: u32 = 0;

pub struct ExtBackgroundEffectManagerV1Global {
    pub name: GlobalName,
}

pub struct ExtBackgroundEffectManagerV1 {
    pub id: ExtBackgroundEffectManagerV1Id,
    pub client: Rc<Client>,
    pub tracker: Tracker<Self>,
    pub version: Version,
}

impl ExtBackgroundEffectManagerV1Global {
    pub fn new(name: GlobalName) -> Self {
        Self { name }
    }

    fn bind_(
        self: Rc<Self>,
        id: ExtBackgroundEffectManagerV1Id,
        client: &Rc<Client>,
        version: Version,
    ) -> Result<(), ExtBackgroundEffectManagerV1Error> {
        let obj = Rc::new(ExtBackgroundEffectManagerV1 {
            id,
            client: client.clone(),
            tracker: Default::default(),
            version,
        });
        track!(client, obj);
        client.add_client_obj(&obj)?;
        client
            .state
            .background_effect_managers
            .set((client.id, id), obj.clone());
        obj.send_capabilities();
        Ok(())
    }
}

impl ExtBackgroundEffectManagerV1 {
    pub fn send_capabilities(&self) {
        let flags = match self
            .client
            .state
            .render_ctx
            .get()
            .is_some_and(|ctx| ctx.supports_background_blur())
        {
            true => CAPABILITY_BLUR,
            false => 0,
        };
        self.client.event(Capabilities {
            self_id: self.id,
            flags,
        });
    }

    fn remove_from_state(&self) {
        self.client
            .state
            .background_effect_managers
            .remove(&(self.client.id, self.id));
    }
}

impl ExtBackgroundEffectManagerV1RequestHandler for ExtBackgroundEffectManagerV1 {
    type Error = ExtBackgroundEffectManagerV1Error;

    fn destroy(&self, _req: Destroy, _slf: &Rc<Self>) -> Result<(), Self::Error> {
        self.remove_from_state();
        self.client.remove_obj(self)?;
        Ok(())
    }

    fn get_background_effect(
        &self,
        req: GetBackgroundEffect,
        _slf: &Rc<Self>,
    ) -> Result<(), Self::Error> {
        let surface = self.client.lookup(req.surface)?;
        if surface.background_effect.is_some() {
            self.client.protocol_error(
                self,
                ERROR_BACKGROUND_EFFECT_EXISTS,
                "wl_surface already has a background effect object",
            );
            return Ok(());
        }
        let effect = Rc::new(ExtBackgroundEffectSurfaceV1::new(
            req.id,
            self.version,
            &surface,
        ));
        track!(self.client, effect);
        self.client.add_client_obj(&effect)?;
        surface.background_effect.set(Some(effect));
        Ok(())
    }
}

global_base!(
    ExtBackgroundEffectManagerV1Global,
    ExtBackgroundEffectManagerV1,
    ExtBackgroundEffectManagerV1Error
);

impl Global for ExtBackgroundEffectManagerV1Global {
    fn version(&self) -> u32 {
        1
    }
}

simple_add_global!(ExtBackgroundEffectManagerV1Global);

object_base! {
    self = ExtBackgroundEffectManagerV1;
    version = self.version;
}

impl Object for ExtBackgroundEffectManagerV1 {
    fn break_loops(self: Rc<Self>) {
        self.remove_from_state();
    }
}

simple_add_obj!(ExtBackgroundEffectManagerV1);

#[derive(Debug, Error)]
pub enum ExtBackgroundEffectManagerV1Error {
    #[error(transparent)]
    ClientError(Box<ClientError>),
}
efrom!(ExtBackgroundEffectManagerV1Error, ClientError);
