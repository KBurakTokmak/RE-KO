// Isolated Win32 test for native movement-before-attack packet ordering and ABI.
#include <windows.h>
#include <cassert>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <iostream>
#include <string>

static unsigned char game[0x300]{}, player[0xc00]{};
static void* active_player=player;
static unsigned char stopped=1;
static unsigned moves=0, attacks=0, expected_moves=0;
static bool moving_expected=false;
void __fastcall move(void* self,void*,uint32_t moving,uint32_t continuous) {
    assert(self==game && moving==unsigned(moving_expected) && continuous==moving);
    if (!moving) assert(stopped==0);
    stopped=moving ? 0 : 1;
    std::memcpy(game+0x28c,player+0x98,12);
    ++moves;
}
void __fastcall attack(void* self,void*,uint32_t target,uint32_t interval,uint32_t distance) {
    assert(self==game && target==12416 && interval==0x3f8b851f && distance==0x3f7851ec);
    assert(moves==expected_moves);
    ++attacks;
}
int main() {
    auto code=static_cast<unsigned char*>(VirtualAlloc(nullptr,4096,MEM_COMMIT|MEM_RESERVE,PAGE_EXECUTE_READWRITE));
    if (!code) return 2;
    std::printf("%u %u %u %u %u\n",unsigned(uintptr_t(code)),unsigned(uintptr_t(&attack)),unsigned(uintptr_t(&move)),unsigned(uintptr_t(&active_player)),unsigned(uintptr_t(&stopped)));
    std::fflush(stdout);
    std::string hex;std::getline(std::cin,hex);
    if (hex.size()>8192 || hex.size()%2) return 3;
    for (size_t i=0;i<hex.size();i+=2) code[i/2]=static_cast<unsigned char>(std::stoul(hex.substr(i,2),nullptr,16));
    FlushInstructionCache(GetCurrentProcess(),code,hex.size()/2);
    using Invoke=void(__thiscall*)(void*,uint32_t,uint32_t,uint32_t);
    auto invoke=reinterpret_cast<Invoke>(code);
    float actual[3]={79.6715f,1.04f,387.9647f}, previous[3]={80.0f,1.0f,389.2f};
    std::memcpy(player+0x98,actual,12);std::memcpy(game+0x28c,previous,12);
    expected_moves=1;invoke(game,12416,0x3f8b851f,0x3f7851ec);
    assert(stopped==1 && moves==1 && attacks==1);
    for (unsigned i=0;i<100;++i) invoke(game,12416,0x3f8b851f,0x3f7851ec);
    assert(moves==1 && attacks==101);
    actual[0]+=1.0f;std::memcpy(player+0x98,actual,12);
    uint32_t moving=3;std::memcpy(player+0x31c,&moving,4);
    moving_expected=true;expected_moves=2;
    invoke(game,12416,0x3f8b851f,0x3f7851ec);
    assert(moves==2 && stopped==0 && attacks==102);
    active_player=nullptr;invoke(game,12416,0x3f8b851f,0x3f7851ec);
    assert(moves==2 && attacks==103);
    std::puts("PASS: changed stops and moving forecasts precede R; unchanged positions send no extra movement; original attack arguments and Win32 stack preserved.");
    return 0;
}
