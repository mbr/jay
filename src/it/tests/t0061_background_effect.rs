use crate::it::test_error::TestResult;
use crate::it::testrun::TestRun;
use crate::rect::Rect;
use std::rc::Rc;

testcase!();

async fn test(run: Rc<TestRun>) -> TestResult {
    let client = run.create_client().await?;
    let surface = client.comp.create_surface().await?;
    let region = client.comp.create_region().await?;
    let effect = client
        .background_effect_manager
        .get_background_effect(&surface)?;

    let committed = Rect::new_sized_saturating(2, 3, 20, 10);
    region.add(committed)?;
    effect.set_blur_region(Some(&region))?;
    region.add(Rect::new_sized_saturating(50, 60, 5, 5))?;
    surface.commit()?;
    client.tran.sync().await;

    let actual = surface.server.blur_region.get().expect("blur region");
    tassert_eq!(actual.rects(), &[committed]);

    effect.destroy()?;
    surface.commit()?;
    client.tran.sync().await;
    tassert!(surface.server.blur_region.is_none());
    Ok(())
}
