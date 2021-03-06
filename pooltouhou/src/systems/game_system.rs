use std::convert::TryFrom;
use std::io::{Error, ErrorKind};

use amethyst::{
    core::{components::Transform},
    derive::SystemDesc,
    ecs::{Entities, Read, RunningTime, System, SystemData, World, Write, WriteStorage},
    ecs::prelude::{Component, DenseVecStorage, Join, ParallelIterator, ParJoin},
    input::VirtualKeyCode,
    renderer::{SpriteRender, Transparent},
    shred::ResourceId,
};
use failure::_core::f32::consts::PI;
use nalgebra::Vector3;

use crate::component::{EnemyBullet, InvertColorAnimation, PlayerBullet};
use crate::CoreStorage;
use crate::handles::TextureHandles;
use crate::render::InvertColorCircle;
use crate::script::{ScriptGameData, ScriptManager};
use crate::script::script_context::ScriptContext;

#[derive(Default)]
pub struct Player {
    move_speed: f32,
    walk_speed: f32,
    radius: f32,
    shoot_cooldown: u8,
}

#[derive(Debug)]
pub enum CollideType {
    Circle(f32)
}

impl CollideType {
    pub fn is_collide_with_point(&self, me: &Vector3<f32>, other: &Vector3<f32>) -> bool {
        match self {
            Self::Circle(r_2) => {
                let x_distance = me.x - other.x;
                let y_distance = me.y - other.y;
                x_distance * x_distance + y_distance * y_distance <= *r_2
            }
        }
    }

    pub fn is_collide_with(&self, me: &Vector3<f32>, other_collide: &CollideType, other: &Vector3<f32>) -> bool {
        match self {
            Self::Circle(r_2) => {
                if *r_2 <= 0.0 {
                    other_collide.is_collide_with_point(me, other)
                } else {
                    //todo: circle collide circle
                    true
                }
            }
        }
    }
}

impl TryFrom<(u8, Vec<f32>)> for CollideType {
    type Error = Error;

    fn try_from((value, args): (u8, Vec<f32>)) -> Result<Self, Self::Error> {
        match value {
            10 => Ok(CollideType::Circle(args[0] * args[0])),
            _ => Err(Error::new(ErrorKind::InvalidData, "No such value for CollideType: ".to_owned() + &*value.to_string()))
        }
    }
}

impl CollideType {
    pub fn get_arg_count(byte: u8) -> usize {
        match byte {
            10 => 1,
            _ => panic!("Not collide byte: {}", byte)
        }
    }
}

impl Player {
    pub fn new(speed: f32) -> Self {
        Self {
            move_speed: speed,
            walk_speed: speed * 0.6,
            radius: 5.0,
            shoot_cooldown: 0,
        }
    }
}

impl Component for Player {
    type Storage = DenseVecStorage<Self>;
}

#[derive(SystemData)]
pub struct GameSystemData<'a> {
    transforms: WriteStorage<'a, Transform>,
    player_bullets: WriteStorage<'a, PlayerBullet>,
    sprite_renders: WriteStorage<'a, SpriteRender>,
    transparent: WriteStorage<'a, Transparent>,
    players: WriteStorage<'a, Player>,
    texture_handles: Read<'a, TextureHandles>,
    core: Write<'a, CoreStorage>,
    entities: Entities<'a>,
    enemies: WriteStorage<'a, crate::component::Enemy>,
    enemy_bullets: WriteStorage<'a, EnemyBullet>,
    animations: (WriteStorage<'a, InvertColorCircle>, WriteStorage<'a, InvertColorAnimation>),
    script_manager: Write<'a, ScriptManager>,
}


#[derive(SystemDesc)]
pub struct GameSystem;

impl<'a> System<'a> for GameSystem {
    type SystemData = GameSystemData<'a>;


    fn run(&mut self, mut data: Self::SystemData) {
        if data.core.tick_sign {
            let mut game_data = ScriptGameData {
                tran: None,
                player_tran: None,
                submit_command: Vec::with_capacity(4),
                script_manager: None,
            };

            process_player(&mut data, &mut game_data);
            game_data.script_manager = Some(&mut data.script_manager);

            data.core.tick_sign = false;
            data.core.tick += 1;
            'bullet_for: for (bullet, bullet_entity) in (&data.player_bullets, &data.entities).join() {
                {
                    let bullet_pos = data.transforms.get(bullet_entity).unwrap().translation();
                    for (enemy, enemy_entity) in (&mut data.enemies, &data.entities).join() {
                        if enemy.hp <= 0.0 {
                            continue;
                        }
                        let enemy_tran = data.transforms.get(enemy_entity).unwrap();
                        let enemy_pos = enemy_tran.translation();
                        if enemy.collide.is_collide_with_point(enemy_pos, bullet_pos) {
                            enemy.hp -= bullet.damage;
                            if enemy.hp <= 0.0 {
                                println!("Anye hp left: 0.0");
                                data.entities.delete(enemy_entity).expect("delete enemy entity failed");
                                boss_die_anime(&data.entities, (&mut data.animations.0, &mut data.animations.1), enemy_pos);
                            } else {
                                println!("Anye hp left: {}", enemy.hp);
                            }
                            data.entities.delete(bullet_entity).expect("delete bullet entity failed");

                            continue 'bullet_for;
                        }
                    }
                }
                let pos = data.transforms.get_mut(bullet_entity).unwrap();
                pos.move_up(30.0);
                if is_out_of_game(pos) {
                    data.entities.delete(bullet_entity).expect("delete bullet entity failed");
                }
            }

            for (enemy_bullet, enemy_entity) in (&mut data.enemy_bullets, &data.entities).join() {
                let enemy_tran = data.transforms.get_mut(enemy_entity).unwrap();
                if is_out_of_game(enemy_tran) {
                    data.entities.delete(enemy_entity).expect("delete enemy entity failed");
                    continue;
                }
                game_data.tran = Some((*enemy_tran).clone());
                enemy_bullet.script.execute_function(&"tick".to_string(), &mut game_data);
                while let Some(x) = game_data.submit_command.pop() {
                    match x {
                        crate::script::ScriptGameCommand::MoveUp(v) => {
                            enemy_tran.move_up(v);
                        }
                        _ => {}
                    }
                }
            }


            for (enemy, enemy_entity) in (&mut data.enemies, &data.entities).join() {
                let enemy_tran = data.transforms.get(enemy_entity).unwrap();
                game_data.tran = Some((*enemy_tran).clone());
                enemy.script.execute_function(&"tick".to_string(), &mut game_data);
                while let Some(x) = game_data.submit_command.pop() {
                    match x {
                        crate::script::ScriptGameCommand::SummonBullet(name, x, y, z, angle, collide, script, args) => {
                            let script_context;
                            if let Some(script) = game_data.script_manager.as_mut().unwrap().get_script(&script) {
                                script_context = ScriptContext::new(script, args);
                            } else {
                                let script = game_data.script_manager.as_mut().unwrap().load_script(&script).unwrap();
                                script_context = ScriptContext::new(script, args);
                            }
                            let mut pos = Transform::default();
                            pos.set_translation_xyz(x, y, z);
                            pos.set_rotation_z_axis(angle / 180.0 * PI);
                            data.entities.build_entity()
                                .with(pos, &mut data.transforms)
                                .with(EnemyBullet { collide, script: script_context }, &mut data.enemy_bullets)
                                .with(data.texture_handles.bullets.get(&*name).unwrap().clone(), &mut data.sprite_renders)
                                .with(Transparent, &mut data.transparent)
                                .build();
                        }
                        _ => {}
                    }
                }
            }
            //tick if end
        }
    }

    fn running_time(&self) -> RunningTime {
        RunningTime::Long
    }
}

fn process_player(data: &mut GameSystemData, game_data: &mut ScriptGameData) {
    if let Some(entity) = data.core.player {
        let player = data.players.get_mut(entity).unwrap();
        let pos = data.transforms.get_mut(entity).unwrap();
        let input = data.core.cur_input.as_ref().unwrap();
        let is_walk = input.pressing.contains(&VirtualKeyCode::LShift);
        let (mov_x, mov_y) = input.get_move(if is_walk {
            player.walk_speed
        } else {
            player.move_speed
        });
        let (raw_x, raw_y) = (pos.translation().x, pos.translation().y);
        pos.set_translation_x((mov_x + raw_x).max(0.0).min(1600.0))
            .set_translation_y((mov_y + raw_y).max(0.0).min(900.0));

        if is_walk {
            data.animations.0.insert(entity, InvertColorCircle {
                pos: (*pos).clone(),
                radius: player.radius,
            }).expect("Insert error");
        } else {
            data.animations.0.remove(entity);
        }
        game_data.player_tran = Some((*pos).clone());

        if player.shoot_cooldown == 0 {
            if input.pressing.contains(&VirtualKeyCode::Z) {
                player.shoot_cooldown = 2;
                let mut pos = (*pos).clone();
                pos.prepend_translation_z(-1.0);
                pos.set_scale(Vector3::new(0.5, 0.5, 1.0));
                data.entities.build_entity()
                    .with(pos, &mut data.transforms)
                    .with(PlayerBullet { damage: 10.0 }, &mut data.player_bullets)
                    .with(data.texture_handles.player_bullet.clone().unwrap(), &mut data.sprite_renders)
                    .with(Transparent, &mut data.transparent)
                    .build();
            }
        } else {
            player.shoot_cooldown -= 1;
        }
        let pos = data.transforms.get(entity).unwrap();

        let collide = CollideType::Circle(player.radius * player.radius);

        let die = (&data.enemy_bullets, &data.entities).par_join().any(|(bullet, enemy_bullet_entity)| {
            let enemy_tran = data.transforms.get(enemy_bullet_entity).unwrap();
            if bullet.collide.is_collide_with(enemy_tran.translation(), &collide, pos.translation()) {
                true
            } else {
                false
            }
        });
        if die {
            boss_die_anime(&mut data.entities, (&mut data.animations.0, &mut data.animations.1), pos.translation());
            data.entities.delete(entity).expect("delete player entity failed");
            data.core.player = None;
        }
    }
}

fn boss_die_anime<'a>(entities: &Entities<'a>,
                      mut animations: (&mut WriteStorage<'a, InvertColorCircle>, &mut WriteStorage<'a, InvertColorAnimation>),
                      enemy_pos: &Vector3<f32>) {
    let last_seconds = 5.0;
    let spread_per_second = 300.0;
    let delay_second = 0.0;
    let mut transform = Transform::default();
    transform.set_translation_x(enemy_pos.x);
    transform.set_translation_y(enemy_pos.y);
    transform.set_translation_z(enemy_pos.z);
    entities.build_entity()
        .with(InvertColorCircle {
            pos: Transform::from(transform.clone()),
            radius: 0.0,
        }, &mut animations.0)
        .with(InvertColorAnimation {
            last_seconds,
            spread_per_second,
            delay_second,
            transform: None,
        }, &mut animations.1)
        .build();
    let last_seconds = 4.75;
    let spread_per_second = 375.0;
    let delay_second = 0.25;
    transform.set_translation_x(enemy_pos.x - 50.0);
    transform.set_translation_y(enemy_pos.y + 50.0);
    entities.build_entity()
        .with(InvertColorAnimation {
            last_seconds,
            spread_per_second,
            delay_second,
            transform: Some(transform.clone()),
        }, &mut animations.1)
        .build();
    transform.set_translation_x(enemy_pos.x + 50.0);
    entities.build_entity()
        .with(InvertColorAnimation {
            last_seconds,
            spread_per_second,
            delay_second,
            transform: Some(transform.clone()),
        }, &mut animations.1)
        .build();
    transform.set_translation_y(enemy_pos.y - 50.0);
    entities.build_entity()
        .with(InvertColorAnimation {
            last_seconds,
            spread_per_second,
            delay_second,
            transform: Some(transform.clone()),
        }, &mut animations.1)
        .build();
    transform.set_translation_x(enemy_pos.x - 50.0);
    entities.build_entity()
        .with(InvertColorAnimation {
            last_seconds,
            spread_per_second,
            delay_second,
            transform: Some(transform.clone()),
        }, &mut animations.1)
        .build();

    let last_seconds = 4.0;
    let spread_per_second = 500.0;
    let delay_second = 1.0;
    transform.set_translation_x(enemy_pos.x);
    transform.set_translation_y(enemy_pos.y);
    entities.build_entity()
        .with(InvertColorAnimation {
            last_seconds,
            spread_per_second,
            delay_second,
            transform: Some(transform),
        }, &mut animations.1)
        .build();
}

pub fn is_out_of_game(tran: &Transform) -> bool {
    let tran = tran.translation();
    tran.x < -100.0 || tran.x > 1700.0 || tran.y > 1000.0 || tran.y < -100.0
}