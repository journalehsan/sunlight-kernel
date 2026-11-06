#![cfg(feature = "js-dom-fixture")]
use rappid_rabbit::{
    account_fixture::{document, AccountFixture, Measure},
    dom_fixture::{FixtureKey as K, FixtureSession},
};
use sunlight_ui::{app::App, widgets::RenderObjectKind, Canvas, Event};
fn eval(app: &mut AccountFixture, script: &str) {
    app.session
        .evaluate(&mut app.render, script, &Measure)
        .unwrap();
}
fn key(app: &mut AccountFixture, key: K) {
    app.session.key(&mut app.render, key, &Measure).unwrap();
}
fn click(app: &mut AccountFixture, id: &str) {
    let node = app.render.dom.get_element_by_id(id).unwrap();
    app.session.click(&mut app.render, node, &Measure).unwrap();
}
fn fill(app: &mut AccountFixture) {
    eval(
        app,
        "email.value='demo@example.test';password.value='invented';",
    );
}
#[test]
fn validation_loading_success_and_failure_under_gc_stress() {
    for success in [true, false] {
        let mut app = AccountFixture::new().unwrap();
        app.session.stress_gc();
        click(&mut app, "submit");
        eval(
            &mut app,
            "if(email.getAttribute('aria-invalid')!=='true') throw 'validation';",
        );
        fill(&mut app);
        click(&mut app, "submit");
        eval(&mut app, "if(!submit.disabled || password.value!=='' || status.className!=='loading') throw 'loading';");
        app.session.collect().unwrap();
        assert!(app
            .session
            .complete(&mut app.render, success, &Measure)
            .unwrap());
        eval(
            &mut app,
            if success {
                "if(status.className!=='success') throw 'success';"
            } else {
                "if(status.className!=='error' || submit.disabled) throw 'failure';"
            },
        );
        assert!(!app
            .session
            .complete(&mut app.render, true, &Measure)
            .unwrap());
    }
}
#[test]
fn unicode_editing_password_masks_and_tab_focus() {
    let mut app = AccountFixture::new().unwrap();
    app.session.stress_gc();
    eval(&mut app, "password.focus();");
    for c in "آ🙂é".chars() {
        key(&mut app, K::Text(c));
    }
    key(&mut app, K::Left);
    key(&mut app, K::Backspace);
    eval(
        &mut app,
        "if(password.value!=='آé') throw 'Unicode backspace';",
    );
    key(&mut app, K::Home);
    key(&mut app, K::Delete);
    key(&mut app, K::End);
    eval(&mut app, "if(password.value!=='é') throw 'Unicode delete';");
    let password = app.render.dom.get_element_by_id("password").unwrap();
    let visual = app
        .render
        .current_scene
        .objects
        .iter()
        .find(|o| {
            o.owner_node_id.0 == password as u64
                && matches!(o.kind, RenderObjectKind::Control { .. })
        })
        .unwrap();
    assert!(matches!(&visual.kind, RenderObjectKind::Control {value,..} if value=="*"));
    key(&mut app, K::Tab(false));
    key(&mut app, K::Space);
    eval(
        &mut app,
        "if(!show.checked || password.type!=='text') throw 'show';",
    );
    key(&mut app, K::Tab(false));
    key(&mut app, K::Space);
    eval(&mut app, "if(!remember.checked) throw 'remember';");
    key(&mut app, K::Tab(true));
    key(&mut app, K::Space);
    eval(
        &mut app,
        "if(show.checked || password.type!=='password') throw 'hide';",
    );
}
#[test]
fn event_order_identity_this_bubbling_cancellation_and_listener_mutation() {
    let mut app = AccountFixture::new().unwrap();
    app.session.stress_gc();
    eval(
        &mut app,
        r#"
        let order='';
        function removed(e){order=order+'bad';}
        function late(e){order=order+'L';}
        email.addEventListener('probe',function(e){
            if(this!==email || e.target!==email || e.currentTarget!==email) throw 'identity';
            order=order+'A'; email.removeEventListener('probe',removed); email.addEventListener('probe',late);
        });
        email.addEventListener('probe',removed);
        form.addEventListener('probe',function(e){if(e.target!==email || e.currentTarget!==form || this!==form) throw 'bubble'; order=order+'B';e.preventDefault();});
        let event=Event('probe',{bubbles:true,cancelable:true});
        if(email.dispatchEvent(event)!==false || !event.defaultPrevented || event.currentTarget!==null || order!=='AB') throw 'first';
        order='';email.dispatchEvent(Event('probe',{bubbles:true,cancelable:true}));
        if(order!=='ALB') throw 'second';
        email.addEventListener('stop',function(e){e.stopPropagation();});
        form.addEventListener('stop',function(){throw 'propagation';});
        email.dispatchEvent(Event('stop',{bubbles:true}));
        show.addEventListener('click',function(e){if(!show.checked) throw 'preactivation';e.preventDefault();});
    "#,
    );
    click(&mut app, "show");
    eval(&mut app, "if(show.checked) throw 'checkbox rollback';");
}
#[test]
fn input_change_submit_order_and_disabled_default_actions() {
    let mut app = AccountFixture::new().unwrap();
    app.session.stress_gc();
    eval(&mut app,"let order='';email.addEventListener('input',function(){order=order+'i';});email.addEventListener('change',function(){order=order+'c';});email.addEventListener('blur',function(){order=order+'b';});password.addEventListener('focus',function(){order=order+'f';});email.focus();");
    key(&mut app, K::Text('a'));
    key(&mut app, K::Tab(false));
    eval(&mut app, "if(order!=='icbf') throw 'ordering';");
    fill(&mut app);
    key(&mut app, K::Enter);
    eval(
        &mut app,
        "if(status.className!=='loading') throw 'enter submit';",
    );
    click(&mut app, "show");
    key(&mut app, K::Text('z'));
    eval(
        &mut app,
        "if(show.checked || password.value!=='') throw 'disabled';",
    );
}
#[test]
fn closure_roots_tasks_cancel_on_generation_change_and_mutations_coalesce() {
    let mut app = AccountFixture::new().unwrap();
    app.session.stress_gc();
    eval(&mut app,"(function(){let captured={text:'alive'};email.addEventListener('test',function(){status.textContent=captured.text;});})();");
    app.session.collect().unwrap();
    let before = app.render.scene_generation;
    eval(&mut app,"email.dispatchEvent(Event('test'));status.className='success';status.setAttribute('data-test','ok');status.textContent='Final';");
    assert_eq!(app.render.scene_generation, before + 1);
    fill(&mut app);
    click(&mut app, "submit");
    let mut next = document(2);
    assert!(app.session.complete(&mut next, true, &Measure).is_err());
    assert!(app.session.complete(&mut next, true, &Measure).is_err());
    drop(app);
    let mut session = FixtureSession::new(&next).unwrap();
    session.stress_gc();
    session.collect().unwrap();
}
#[test]
fn capture_and_unsupported_selectors_are_rejected() {
    let mut app = AccountFixture::new().unwrap();
    for script in [
        "email.addEventListener('x',function(){},true)",
        "email.addEventListener('x',function(){},{once:true})",
        "document.querySelector('form input')",
    ] {
        assert!(app
            .session
            .evaluate(&mut app.render, script, &Measure)
            .is_err());
    }
}
#[test]
fn ui_click_keyboard_and_tick_use_the_same_session() {
    let mut app = AccountFixture::new().unwrap();
    let id = app.render.dom.get_element_by_id("email").unwrap();
    let rect = app
        .render
        .current_scene
        .objects
        .iter()
        .find(|o| {
            o.owner_node_id.0 == id as u64 && matches!(o.kind, RenderObjectKind::Control { .. })
        })
        .unwrap()
        .bounds;
    assert!(app.update(Event::Click {
        x: rect.x + 5,
        y: rect.y + 5
    }));
    assert!(app.update(Event::Key('آ')));
    eval(&mut app, "if(email.value!=='آ') throw 'UI text';");
    fill(&mut app);
    click(&mut app, "submit");
    assert!(app.update(Event::Tick));
    eval(
        &mut app,
        "if(status.className!=='success') throw 'UI completion';",
    );
    let mut pixels = vec![0; 640 * 700];
    app.view(
        &mut Canvas::new(&mut pixels, 640, 640, 700),
        &sunlight_ui::Theme::sunlight_light(),
    );
}

#[test]
fn reentrant_event_getters_and_blur_are_safe() {
    let mut app = AccountFixture::new().unwrap();
    app.session.stress_gc();
    eval(&mut app, "email.focus();email.blur();const probe=Event('probe');Object.defineProperty(probe,'type',{get:function(){email.value='safe';return 'probe';}});email.dispatchEvent(probe);");
    eval(
        &mut app,
        "if(email.value!=='safe') throw 'reentrant getter';",
    );
    let node = app.render.dom.get_element_by_id("password").unwrap();
    eval(&mut app, "password.value='SYNTHETIC_SECRET_927';");
    assert!(!format!("{:?}", app.render.current_scene).contains("SYNTHETIC_SECRET_927"));
    assert!(app.render.dom.get_attribute(node, "value").is_none());
    let replacement = std::sync::Arc::new((*app.render.dom).clone());
    app.render.dom = replacement;
    assert!(app
        .session
        .complete(&mut app.render, true, &Measure)
        .is_err());
}

#[test]
fn tab_order_includes_secondary_link_and_keyboard_activation() {
    let mut app = AccountFixture::new().unwrap();
    app.session.stress_gc();
    key(&mut app, K::Tab(true));
    eval(&mut app,"if(document.activeElement!==document.getElementById('about')) throw 'reverse initial tab';");
    key(&mut app, K::Enter);
    eval(&mut app,"if(status.textContent!=='This isolated fixture has no provider connection or credential storage.') throw 'link activation';");
    key(&mut app, K::Tab(false));
    eval(&mut app, "if(document.activeElement!==email) throw 'wrap';");
}

#[test]
fn dispatch_depth_and_task_callback_roots_are_bounded() {
    let mut app = AccountFixture::new().unwrap();
    app.session.stress_gc();
    eval(&mut app,"function recurse(){email.dispatchEvent(Event('recurse'));} email.addEventListener('recurse',recurse);");
    assert!(app
        .session
        .evaluate(
            &mut app.render,
            "email.dispatchEvent(Event('recurse'));",
            &Measure
        )
        .is_err());
    eval(&mut app,"email.removeEventListener('recurse',recurse);let previousHandle=fixtureAuthorize(function(){status.textContent='task survived';});");
    app.session.collect().unwrap();
    app.session
        .complete(&mut app.render, true, &Measure)
        .unwrap();
    eval(&mut app,"if(status.textContent!=='task survived') throw 'task root';if(fixtureAuthorize(function(){})<=previousHandle) throw 'request identity';");
    app.session.cancel_pending();
}

#[test]
fn native_modifier_snapshots_and_grapheme_deletion_under_gc() {
    let mut app = AccountFixture::new().unwrap();
    app.session.stress_gc();
    eval(&mut app, "if(document.activeElement!==email) throw 'initial focus';let shifted=false;email.addEventListener('keydown',function(e){shifted=e.shiftKey;});");
    app.update(Event::key_press(0x2a, true, true, false, false, false));
    app.update(Event::Key('é'));
    eval(&mut app, "if(!shifted) throw 'native Shift';");
    app.update(Event::key_press(0x2a, false, false, false, false, false));
    app.update(Event::Key('x'));
    eval(
        &mut app,
        "if(shifted) throw 'released Shift';password.focus();",
    );
    for c in "aé\u{301}👩\u{200d}💻".chars() {
        key(&mut app, K::Text(c));
    }
    key(&mut app, K::Backspace);
    eval(
        &mut app,
        "if(password.value!=='aé́') throw 'emoji cluster deletion';",
    );
    key(&mut app, K::Left);
    key(&mut app, K::Delete);
    eval(
        &mut app,
        "if(password.value!=='a') throw 'combining cluster deletion';",
    );
    key(&mut app, K::Home);
    key(&mut app, K::Delete);
    eval(&mut app, "if(password.value!=='') throw 'empty';");
}
