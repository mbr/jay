use crate::it::test_error::TestError;
use crate::it::test_error::TestResult;
use crate::it::test_ifs::test_background_effect_surface::TestBackgroundEffectSurface;
use crate::it::test_ifs::test_surface::TestSurface;
use crate::it::test_object::TestObject;
use crate::it::test_transport::TestTransport;
use crate::it::test_utils::test_expected_event::TEEH;
use crate::it::testrun::ParseFull;
use crate::utils::buffd::MsgParser;
use crate::wire::ExtBackgroundEffectManagerV1Id;
use crate::wire::ext_background_effect_manager_v1::*;
use std::cell::Cell;
use std::rc::Rc;

pub struct TestBackgroundEffectManager {
    pub id: ExtBackgroundEffectManagerV1Id,
    pub tran: Rc<TestTransport>,
    pub capabilities: TEEH<u32>,
}

impl TestBackgroundEffectManager {
    pub fn new(tran: &Rc<TestTransport>) -> Self {
        Self {
            id: tran.id(),
            tran: tran.clone(),
            capabilities: Default::default(),
        }
    }

    pub fn get_background_effect(
        &self,
        surface: &TestSurface,
    ) -> TestResult<Rc<TestBackgroundEffectSurface>> {
        let effect = Rc::new(TestBackgroundEffectSurface {
            id: self.tran.id(),
            tran: self.tran.clone(),
            destroyed: Cell::new(false),
        });
        self.tran.add_obj(effect.clone())?;
        self.tran.send(GetBackgroundEffect {
            self_id: self.id,
            id: effect.id,
            surface: surface.id,
        })?;
        Ok(effect)
    }

    fn handle_capabilities(&self, parser: MsgParser<'_, '_>) -> Result<(), TestError> {
        let event = Capabilities::parse_full(parser)?;
        self.capabilities.push(event.flags);
        Ok(())
    }
}

test_object! {
    TestBackgroundEffectManager, ExtBackgroundEffectManagerV1;

    CAPABILITIES => handle_capabilities,
}

impl TestObject for TestBackgroundEffectManager {}
