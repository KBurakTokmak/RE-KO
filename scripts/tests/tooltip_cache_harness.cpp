// Isolated Win32 ABI test: no game files or process injection.
#include <windows.h>
#include <cstdio>
#include <cstdint>
#include <string>
#include <iostream>
#include <cstring>
#include <cassert>
struct Object { void** table; char pad[0x88-4]; RECT rect; char tail[0xa8-0x98]; unsigned char visible; };
struct Item { char pad[0x38]; uint32_t base, ext, count, durability; };
static int builds=0, renders=0;
static int slash_calls=0, ordinary_calls=0;
__declspec(naked) void slash() { __asm inc slash_calls __asm ret }
__declspec(naked) void ordinary() { __asm inc ordinary_calls __asm ret }
void command(void* entry,const char* text) {
    struct String { uint32_t allocator;const char* text;uint32_t length,capacity; };
    String value{0,text,uint32_t(std::strlen(text)),uint32_t(std::strlen(text))};
    __asm mov eax,text
    __asm lea esi,value
    __asm mov edx,entry
    __asm call edx
}
void __fastcall position(Object* o,void*,int x,int y) {
    int width=o->rect.right-o->rect.left,height=o->rect.bottom-o->rect.top;
    o->rect={x,y,x+width,y+height};
}
void __fastcall render(Object*,void*) { ++renders; }
void __fastcall show(Object* o,void*,uint32_t visible) { o->visible=static_cast<unsigned char>(visible); }
void __fastcall original(Object* o,void*,uint32_t,uint32_t,Item*,uint32_t,uint32_t,uint32_t) {
    ++builds; ++renders; o->visible=1; o->rect={0,0,200,300};
}
int main() {
    auto data=static_cast<unsigned char*>(VirtualAlloc(nullptr,8192,MEM_COMMIT|MEM_RESERVE,PAGE_EXECUTE_READWRITE));
    if(!data) return 2;
    unsigned char player[0x930]{};void* player_pointer=player;int viewport[2]={1024,768};
    std::printf("%u %u %u %u %u\n",unsigned(uintptr_t(data+4096)),unsigned(uintptr_t(data)),unsigned(uintptr_t(&original)),unsigned(uintptr_t(&player_pointer)),unsigned(uintptr_t(viewport)));
    std::fflush(stdout);
    std::string hex; std::getline(std::cin,hex);
    if(hex.size()>8192 || hex.size()%2) return 3;
    for(size_t i=0;i<hex.size();i+=2) data[4096+i/2]=static_cast<unsigned char>(std::stoul(hex.substr(i,2),nullptr,16));
    FlushInstructionCache(GetCurrentProcess(),data+4096,hex.size()/2);
    void* table[28]{};table[27]=reinterpret_cast<void*>(&render);table[17]=reinterpret_cast<void*>(&show);
    Object o{};o.table=table;table[9]=reinterpret_cast<void*>(&position);
    Item item{};item.base=111210000;item.ext=7;item.count=1;item.durability=10000;
    using Display=void(__thiscall*)(Object*,uint32_t,uint32_t,Item*,uint32_t,uint32_t,uint32_t);
    auto display=reinterpret_cast<Display>(data+4096);
    for(int i=0;i<100;++i) {o.visible=0;display(&o,100,200,&item,0,1,0);assert(o.visible==1);}
    assert(renders==100 && builds==1);
    ++item.durability;display(&o,100,200,&item,0,1,0);assert(builds==2);
    ++item.count;display(&o,100,200,&item,0,1,0);assert(builds==3);
    ++item.ext;display(&o,100,200,&item,0,1,0);assert(builds==4);
    for(unsigned x=101;x<700;++x) {o.visible=0;display(&o,x,500,&item,0,1,0);}
    assert(builds==4 && o.rect.left==727 && o.rect.top==202);
    display(&o,1000,760,&item,0,1,0);assert(builds==4 && o.rect.left==802 && o.rect.top==462);
    display(&o,10,10,&item,0,1,0);assert(builds==4 && o.rect.left==38 && o.rect.top==12);
    ++player[0x4ec];display(&o,10,10,&item,0,1,0);assert(builds==5);
    ++player[0x8bc];display(&o,10,10,&item,0,1,0);assert(builds==6);
    display(&o,10,10,&item,1,1,0);assert(builds==7);
    display(&o,10,10,nullptr,1,1,0);assert(builds==8);
    display(&o,10,10,nullptr,1,1,0);assert(builds==9);
    auto entry=data+5000;
    std::printf("%u %u %u\n",unsigned(uintptr_t(entry)),unsigned(uintptr_t(&slash)),unsigned(uintptr_t(&ordinary)));
    std::fflush(stdout);
    std::getline(std::cin,hex);
    if(hex.size()>4000 || hex.size()%2) return 4;
    for(size_t i=0;i<hex.size();i+=2) entry[i/2]=static_cast<unsigned char>(std::stoul(hex.substr(i,2),nullptr,16));
    FlushInstructionCache(GetCurrentProcess(),entry,hex.size()/2);
    for(auto text:{"/autoloot","/autoloot on","/autoloot off","autoloot","hello"}) command(entry,text);
    for(auto text:{"/town","/auto","/autolootx","/autoloot/off","/"}) command(entry,text);
    assert(ordinary_calls==5 && slash_calls==5);
    std::puts("PASS: Win32 thiscall stack, per-frame hide/show, cursor movement and screen edges, stat/item invalidation and null item.");
    std::puts("PASS: autoloot forwarding, argument boundary and other native slash commands preserved.");
    return 0;
}
