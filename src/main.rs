extern crate tilelib;

use tilelib::tile::{TilePosition, TileCollection, Tile};
use tilelib::board::RectangularBoard;
use tilelib::render::render_single_tiling_from_vec;

use std::collections::{HashSet,HashMap};
use std::sync::{Arc, RwLock};
use rayon::prelude::*;
use serde_derive::Serialize;
use clap::{Arg, App};
use rand::Rng;

#[macro_use]
extern crate clap;

#[derive(Debug, Serialize)]
pub struct BoardGraph {
    // The nodes in our graph are boards - we store there here inside a vec
    //// so that we dont have Rc<RefCell<..>> all over the place
    nodes_arena : Vec<RectangularBoard>,

    #[serde(skip_serializing)]
    nodes_arena_index : usize,

    // An edge in our graph indicates that it is possible to get from one board state
    // to another by placing down a tile.
    edges : HashMap<usize, HashSet<usize>>,

    // If a complete tiling of an initial board (index 0 in nodes_arena)
    complete_index : Option<usize>,
}

impl BoardGraph {
    pub fn new() -> Self {
        BoardGraph {
            nodes_arena : Vec::new(),
            nodes_arena_index : 0,
            edges : HashMap::new(),
            complete_index : None,
        }
    }

    pub fn add_node(&mut self, val : RectangularBoard) -> usize {
        self.nodes_arena.push(val);

        self.nodes_arena_index += 1;
        self.nodes_arena_index - 1
    }

    pub fn add_edge(&mut self, s : usize, t : usize) {
        assert!(s < self.nodes_arena_index && t < self.nodes_arena_index);

        self.edges.entry(s).or_insert_with(HashSet::new).insert(t);
    }
}


pub struct Tiler {
    tiles: TileCollection,
    board: RectangularBoard,
}

impl Tiler {
    pub fn new(tiles : TileCollection, board : RectangularBoard) -> Self {
        Tiler {
            tiles,
            board,
        }
    }
}

pub fn get_single_tiling(tiler : Tiler) -> Option<Vec<RectangularBoard>> {
    let mut stack = Vec::new();
    stack.push(vec![tiler.board.clone()]);

    let mut completed_tilings = Vec::new();

    while let Some(tvec) = stack.pop() {
        let current_board = tvec.last().unwrap();

        if let Some(p) = current_board.get_unmarked_position(&tiler.tiles.tiles) {
            let mut fitting_tiles = Vec::new();

            for tile in tiler.tiles.tiles.iter() {
                for start_index in 0..=tile.directions.len() {
                    if let Some(tile_position) = current_board.tile_fits_at_position(tile, p, start_index) {
                        if !fitting_tiles.contains(&tile_position) {
                            fitting_tiles.push(tile_position);
                        }
                    }
                }
            }

            for tp in fitting_tiles {
                let mut marked_board = current_board.clone();
                marked_board.mark_tile_at_position(tp);

                let is_all_marked = marked_board.is_all_marked();

                let mut new_tvec = tvec.clone();
                new_tvec.push(marked_board);

                if is_all_marked {
                    completed_tilings.push(new_tvec);
                } else {
                    stack.push(new_tvec);
                }
            }

            // Stop looking for tilings if we've already found 1000.
            // TODO: maybe make this number configurable
            if completed_tilings.len() >= 1000 {
                break;
            }
        }
    }

    if !completed_tilings.is_empty() {
        // Select a random solution from those already found
        let solution_index = rand::thread_rng().gen_range(0, completed_tilings.len());
        return Some(completed_tilings[solution_index].clone());
    }

    None
}


pub fn count_tilings(tiler : Tiler) -> u64 {
    // at each stage, keep track of the counts
    let mut counter = HashMap::new();
    counter.insert(tiler.board.clone(), 1);

    let mut counter = Arc::new(RwLock::new(counter));

    let tiler_ref = Arc::new(RwLock::new(tiler.tiles.tiles.clone()));

    let mut stack = HashSet::new();
    stack.insert(tiler.board.clone());

    let mut completed_board = None;

    while !stack.is_empty() {
        let handles = stack.par_iter().map(|b| {
            let current_tiler_ref = Arc::clone(&tiler_ref);
            let current_counter_ref = Arc::clone(&counter);

            let mut next_boards = HashSet::new();
            let mut completed_boards = HashSet::new();
            let mut count_updates = HashMap::new();

            if let Some(p) = b.get_unmarked_position(&current_tiler_ref.read().unwrap()) {
                let mut fitting_tiles = Vec::new();

                for tile in current_tiler_ref.read().unwrap().iter() {
                    for start_index in 0..=tile.directions.len() {
                        if let Some(tile_position) = b.tile_fits_at_position(tile, p, start_index) {
                            if !fitting_tiles.contains(&tile_position) {
                                fitting_tiles.push(tile_position);
                            }
                        }
                    }
                }

                for tp in fitting_tiles {
                    let mut marked_board = b.clone();
                    marked_board.mark_tile_at_position(tp);

                    // how many tilings does the previous state have?
                    let current_count = current_counter_ref.read().unwrap()[&b];

                    *count_updates.entry(marked_board.clone()).or_insert(0) += current_count;

                    if marked_board.is_all_marked() {
                        completed_boards.insert(marked_board);
                    } else {
                        next_boards.insert(marked_board);
                    }
                }
            }

            (next_boards, completed_boards, count_updates)
        }).collect::<Vec<_>>();

        stack = HashSet::new();
        counter = Arc::new(RwLock::new(HashMap::new()));

        for (next_boards, completed_boards, count_updates) in handles {
            stack.extend(next_boards);

            {
                let mut write = counter.write().unwrap();

                // update the counts
                for (board, cnt) in count_updates {
                    let entry = write.entry(board).or_insert(0);
                    (*entry) += cnt;
                }
            }

            for board in completed_boards {
                completed_board = Some(board);
            }
        }
    }

    if let Some(completed_board) = completed_board {
        counter.read().unwrap()[&completed_board]
    } else {
        0
    }
}

pub fn compute_boardgraph(tiler : Tiler) -> BoardGraph {
    let mut hashm = HashMap::new();
    let mut completed_boards = HashSet::new();

    let mut board_graph = BoardGraph::new();
    let mut board_graph_hashmap = HashMap::new();

    let board = tiler.board.clone();
    let tiles = tiler.tiles.clone();

    // add the starting board to our hashmap & graoh
    board_graph_hashmap.insert(board.clone(), board_graph.add_node(board.clone()));

    let mut stack = HashSet::new();
    stack.insert(board);

    while !stack.is_empty() {
        let handles : Vec<_> = stack.into_par_iter().map(|b| {
            let mut to_stack = Vec::new();
            let mut to_hash = HashMap::new();
            let mut to_completed = Vec::new();

            if let Some(p) = b.get_unmarked_position(&tiles.tiles) {
                let mut fitting_tiles: Vec<TilePosition> = Vec::new();

                for tile in &tiles.tiles {
                    for start_index in 0..=tile.directions.len() {
                        if let Some(tp) = b.tile_fits_at_position(tile, p, start_index) {
                            if !fitting_tiles.contains(&tp) {
                                fitting_tiles.push(tp);
                            }
                        }
                    }
                }

                let mut next_board = HashMap::new();

                // now for each fitting tile, mark our board with this tile & add it to the stack
                for tp in fitting_tiles {
                    let mut marked_board = b.clone();
                    marked_board.mark_tile_at_position(tp.clone());

                    next_board.entry(marked_board).or_insert_with(Vec::new).push(tp);
                }


                for (k, _) in next_board.into_iter() {
                    to_hash.entry(k.clone()).or_insert_with(Vec::new).push(b.clone());
                    if k.is_all_marked() {
                        to_completed.push(k);
                    } else {
                        to_stack.push(k);
                    }
                }
            }

            (to_stack, to_hash, to_completed)
        }).collect();

        // now merge the results back in
        stack = HashSet::new();

        for (to_stack, to_hash, to_completed) in handles {
            // merge everything in
            stack.extend(to_stack);
            completed_boards.extend(to_completed);

            // merge in hashm
            for (k,v) in to_hash {
                // k = target, v = sources?

                if !board_graph_hashmap.contains_key(&k) {
                    board_graph_hashmap.insert(k.clone(), board_graph.add_node(k.clone()));
                }

                let node = board_graph_hashmap[&k];

                // now insert an edge from v to k
                for p in &v {
                    board_graph.add_edge(board_graph_hashmap[p], node);
                }

                let entry = hashm.entry(k).or_insert_with(Vec::new);
                (*entry).extend(v);
            }
        }
    }

    for complete in completed_boards {
        board_graph.complete_index = Some(board_graph_hashmap[&complete]);
    }
    board_graph
}

arg_enum!{
    #[derive(Debug, Copy, Clone)]
    pub enum BoardType {
        Rectangle,
        LBoard,
        TBoard,
    }
}

arg_enum!{
    #[derive(Debug, Copy, Clone)]
    pub enum TileType {
        LTile,
        TTile
    }
}



fn main() {
    let matches = App::new("rs-tiler")
        .version("1.0")
        .author("Robert Usher")
        .about("Computes various tilings")
        .arg(Arg::with_name("board_size")
                 .help("The size of the board to tile")
                 .index(1)
                 .required(true))
        .arg(Arg::with_name("width")
                 .short("w")
                 .long("width")
                 .takes_value(true)
                 .help("The (optional) width of the board"))
        .arg(Arg::with_name("board_type")
                 .help("The type of board to use")
                 .possible_values(&BoardType::variants())
                 .default_value("LBoard")
                 .index(3))
        .arg(Arg::with_name("board_scale")
                 .help("The board scale to use, if using an LBoard")
                 .long("scale")
                 .default_value("1"))
        .arg(Arg::with_name("tile_type")
                 .help("The type of tile to use")
                 .possible_values(&TileType::variants())
                 .default_value("LTile")
                 .index(4))
        .arg(Arg::with_name("tile_size")
                 .help("The size of the tile")
                 .index(2)
                 .required(true))
        .arg(Arg::with_name("single")
                 .short("s")
                 .long("single")
                 .help("Computes a single tiling")
                 .conflicts_with("count")
                 .conflicts_with("graph"))
        .arg(Arg::with_name("count")
                 .short("c")
                 .long("count")
                 .help("Counts all tilings")
                 .conflicts_with("single")
                 .conflicts_with("graph"))
        .arg(Arg::with_name("graph")
                 .short("g")
                 .long("graph")
                 .help("Computes the full tilings graph")
                 .conflicts_with("count")
                 .conflicts_with("single"))
        .arg(Arg::with_name("scaling")
                 .long("scaling")
                 .help("Computes the tiling count for different values of the scale parameter")
                 .conflicts_with("graph")
                 .conflicts_with("count")
                 .conflicts_with("single"))
        .get_matches();

    let board_type = value_t!(matches.value_of("board_type"), BoardType).unwrap_or_else(|e| e.exit());
    let tile_type = value_t!(matches.value_of("tile_type"), TileType).unwrap_or_else(|e| e.exit());
    let board_size = value_t!(matches.value_of("board_size"),usize).unwrap_or_else(|e| e.exit());

    let board_width = if matches.is_present("width") {
        value_t!(matches.value_of("width"), usize).unwrap_or_else(|e| e.exit())
    } else {
        board_size
    };

    let tile_size = value_t!(matches.value_of("tile_size"), usize).unwrap_or_else(|e| e.exit());
    let board_scale = value_t!(matches.value_of("board_scale"), usize).unwrap_or_else(|e| e.exit());

    // Create the tile & tilecollection specified by the user
    let tile = match tile_type {
        TileType::LTile => {
            Tile::l_tile(tile_size)
        },
        TileType::TTile => {
            Tile::t_tile(tile_size)
        },
    };

    let tiles = TileCollection::from(tile);

    // A closure to create a board based on specified options
    let make_board = |board_type : BoardType, board_size : usize, board_width : usize, board_scale : usize| {
        match board_type {
            BoardType::Rectangle => RectangularBoard::new(board_width, board_size),
            BoardType::LBoard => RectangularBoard::l_board(board_size, board_scale),
            BoardType::TBoard => RectangularBoard::t_board(board_size, board_scale),
        }
    };

    let board = make_board(board_type, board_size, board_width,board_scale);

    if matches.is_present("scaling") {
        let mut board_scale : usize = 1;

        loop {
            let tiler = Tiler::new(tiles.clone(), make_board(board_type, board_size, board_width,board_scale));
            println!("scale({}), {} tilings", board_scale, count_tilings(tiler));
            board_scale += 1;
        }
    } else if matches.is_present("count") {
        dbg!(count_tilings(Tiler::new(tiles, board)));
    } else if matches.is_present("single") {
        let tiler = Tiler::new(tiles, board);

        // render a single tiling
        let tiling = get_single_tiling(tiler);

        if let Some(tiling) = tiling {
            println!("{}", render_single_tiling_from_vec(tiling));
        } else {
            println!("No tilings found!");
        }
    } else if matches.is_present("graph") {
        let tiler = Tiler::new(tiles, board);

        // compute the entire boardgraph for this tiler
        let board_graph = compute_boardgraph(tiler);

        println!("{}", serde_json::to_string(&board_graph).unwrap());
    }
}