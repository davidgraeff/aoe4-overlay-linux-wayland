pub mod consts {
    use image::math::Rect;

    pub const STAT_RECT: Rect = Rect{ x: 0, y: 0, width: 80, height: 34 };
    pub const VILLAGER_ICON_AREA: Rect = Rect{ x: 0, y: 0, width: 250, height: 80 };
    pub const AREA_Y_OFFSET: f32 = -486.0;
    pub const AREA_HEIGHT: i32 = -AREA_Y_OFFSET as i32;
    pub const AREA_WIDTH: i32 = 267;

    #[derive(Debug, Clone)]
    pub struct Aoe4StatPos{
        pub x: f32,
        pub y: f32,
        pub name: &'static str,
    }

    pub const AOE4_STATS_POS: [Aoe4StatPos; 10] = [
        Aoe4StatPos { x: 50.0, y: 190.0 + AREA_Y_OFFSET, name: "Pop" },
        Aoe4StatPos { x: 50.0, y: 265.0 + AREA_Y_OFFSET, name: "Food" },
        Aoe4StatPos { x: 50.0, y: 318.0 + AREA_Y_OFFSET, name: "Wood" },
        Aoe4StatPos { x: 50.0, y: 369.0 + AREA_Y_OFFSET, name: "Gold" },
        Aoe4StatPos { x: 50.0, y: 421.0 + AREA_Y_OFFSET, name: "Stone" },

        Aoe4StatPos { x: 187.0, y: 190.0 + AREA_Y_OFFSET, name: "Idle" },
        Aoe4StatPos { x: 187.0, y: 262.0 + AREA_Y_OFFSET, name: "Food Worker" },
        Aoe4StatPos { x: 187.0, y: 315.0 + AREA_Y_OFFSET, name: "Wood Worker" },
        Aoe4StatPos { x: 187.0, y: 366.0 + AREA_Y_OFFSET, name: "Gold Worker" },
        Aoe4StatPos { x: 187.0, y: 419.0 + AREA_Y_OFFSET, name: "Stone Worker" },
    ];
}

pub mod image_analyzer;
