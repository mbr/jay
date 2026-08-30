use crate::it::test_error::TestError;
use crate::it::test_ifs::test_region::TestRegion;
use crate::it::test_object::TestObject;
use crate::it::test_transport::TestTransport;
use crate::wire::ExtBackgroundEffectSurfaceV1Id;
use crate::wire::WlRegionId;
use crate::wire::ext_background_effect_surface_v1::*;
use std::cell::Cell;
use std::rc::Rc;

pub struct TestBackgroundEffectSurface {
    pub id: ExtBackgroundEffectSurfaceV1Id,
    pub tran: Rc<TestTransport>,
    pub destroyed: Cell<bool>,
}

impl TestBackgroundEffectSurface {
    pub fn destroy(&self) -> Result<(), TestError> {
        if !self.destroyed.replace(true) {
            self.tran.send(Destroy { self_id: self.id })?;
        }
        Ok(())
    }

    pub fn set_blur_region(&self, region: Option<&TestRegion>) -> Result<(), TestError> {
        self.tran.send(SetBlurRegion {
            self_id: self.id,
            region: region.map_or(WlRegionId::NONE, |region| region.id),
        })
    }
}

impl Drop for TestBackgroundEffectSurface {
    fn drop(&mut self) {
        let _ = self.destroy();
    }
}

test_object! {
    TestBackgroundEffectSurface, ExtBackgroundEffectSurfaceV1;
}

impl TestObject for TestBackgroundEffectSurface {}
